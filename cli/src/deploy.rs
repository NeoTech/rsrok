use crate::cloudflare_config::CfAccount;
use crate::worker_bundle;
use reqwest::multipart;
use serde::Deserialize;
use std::fmt;

const CF_API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug)]
pub enum DeployError {
    Http(reqwest::Error),
    Api { errors: Vec<ApiError> },
    NoSubdomain,
}

#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub code: u32,
    pub message: String,
}

impl fmt::Display for DeployError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeployError::Http(e) => write!(f, "HTTP error: {e}"),
            DeployError::Api { errors } => {
                write!(f, "Cloudflare API errors: ")?;
                for (i, e) in errors.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "[{}] {}", e.code, e.message)?;
                }
                Ok(())
            }
            DeployError::NoSubdomain => write!(
                f,
                "could not determine workers.dev subdomain for this account"
            ),
        }
    }
}

impl std::error::Error for DeployError {}

impl From<reqwest::Error> for DeployError {
    fn from(e: reqwest::Error) -> Self {
        DeployError::Http(e)
    }
}

#[derive(Deserialize)]
struct CfResponse<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
    result: Option<T>,
}

#[derive(Deserialize)]
struct SubdomainResult {
    subdomain: String,
}

/// Deploy the embedded Cloudflare Worker and return the public URL.
/// If `auth_token` is Some and non-empty, it is stored as the `AUTH_TOKEN` secret
/// on the deployed worker (enforces CLI registration auth).
pub async fn deploy_worker(
    cf: &CfAccount,
    worker_name: &str,
    auth_token: Option<&str>,
) -> Result<String, DeployError> {
    let client = reqwest::Client::new();

    // Step 1: Delete any existing script so we start tag-fresh (ignore 404)
    delete_script(&client, cf, worker_name).await?;

    // Step 2: Upload the Worker script + WASM via multipart PUT
    upload_script(&client, cf, worker_name).await?;

    // Step 3: Enable workers.dev subdomain for this script
    enable_subdomain(&client, cf, worker_name).await?;

    // Step 4: Store AUTH_TOKEN secret if provided
    if let Some(token) = auth_token {
        if !token.is_empty() {
            put_secret(&client, cf, worker_name, "AUTH_TOKEN", token).await?;
        }
    }

    // Step 5: Get the account's workers.dev subdomain
    let subdomain = get_subdomain(&client, cf).await?;

    Ok(format!("https://{worker_name}.{subdomain}.workers.dev"))
}

async fn put_secret(
    client: &reqwest::Client,
    cf: &CfAccount,
    worker_name: &str,
    secret_name: &str,
    secret_value: &str,
) -> Result<(), DeployError> {
    let url = format!(
        "{CF_API_BASE}/accounts/{}/workers/scripts/{worker_name}/secrets",
        cf.account_id
    );

    let resp: CfResponse<serde_json::Value> = client
        .put(&url)
        .bearer_auth(&cf.api_token)
        .json(&serde_json::json!({
            "name": secret_name,
            "text": secret_value,
            "type": "secret_text"
        }))
        .send()
        .await?
        .json()
        .await?;

    if !resp.success {
        return Err(DeployError::Api { errors: resp.errors });
    }

    Ok(())
}

async fn delete_script(
    client: &reqwest::Client,
    cf: &CfAccount,
    worker_name: &str,
) -> Result<(), DeployError> {
    let url = format!(
        "{CF_API_BASE}/accounts/{}/workers/scripts/{worker_name}",
        cf.account_id
    );

    let resp = client
        .delete(&url)
        .bearer_auth(&cf.api_token)
        .send()
        .await?;

    // 404 means it didn't exist — that's fine, we wanted it gone anyway
    if resp.status().as_u16() == 404 {
        return Ok(());
    }

    let body: CfResponse<serde_json::Value> = resp.json().await?;
    if !body.success {
        return Err(DeployError::Api { errors: body.errors });
    }

    Ok(())
}

async fn upload_script(
    client: &reqwest::Client,
    cf: &CfAccount,
    worker_name: &str,
) -> Result<(), DeployError> {
    let metadata = serde_json::json!({
        "main_module": "index.js",
        "compatibility_date": worker_bundle::COMPATIBILITY_DATE,
        "compatibility_flags": ["nodejs_compat"],
        "bindings": [
            {
                "type": "durable_object_namespace",
                "name": "TUNNEL_REGISTRY",
                "class_name": "TunnelRegistry"
            },
            {
                "type": "durable_object_namespace",
                "name": "MODE_REGISTRY",
                "class_name": "ModeRegistry"
            }
        ],
        "migrations": {
                "new_tag": "v1",
                "steps": [
                    {
                        "new_sqlite_classes": ["TunnelRegistry", "ModeRegistry"]
                    }
                ]
            }
    });

    let form = multipart::Form::new()
        .part(
            "metadata",
            multipart::Part::text(metadata.to_string())
                .mime_str("application/json")?,
        )
        .part(
            "index.js",
            multipart::Part::bytes(worker_bundle::WORKER_SCRIPT.to_vec())
                .file_name("index.js")
                .mime_str("application/javascript+module")?,
        )
        .part(
            worker_bundle::WASM_MODULE_NAME.to_string(),
            multipart::Part::bytes(worker_bundle::WORKER_WASM.to_vec())
                .file_name(worker_bundle::WASM_MODULE_NAME.to_string())
                .mime_str("application/wasm")?,
        );

    let url = format!(
        "{CF_API_BASE}/accounts/{}/workers/scripts/{worker_name}",
        cf.account_id
    );

    let resp: CfResponse<serde_json::Value> = client
        .put(&url)
        .bearer_auth(&cf.api_token)
        .multipart(form)
        .send()
        .await?
        .json()
        .await?;

    if !resp.success {
        return Err(DeployError::Api { errors: resp.errors });
    }

    Ok(())
}

async fn enable_subdomain(
    client: &reqwest::Client,
    cf: &CfAccount,
    worker_name: &str,
) -> Result<(), DeployError> {
    let url = format!(
        "{CF_API_BASE}/accounts/{}/workers/scripts/{worker_name}/subdomain",
        cf.account_id
    );

    let resp: CfResponse<serde_json::Value> = client
        .post(&url)
        .bearer_auth(&cf.api_token)
        .json(&serde_json::json!({ "enabled": true }))
        .send()
        .await?
        .json()
        .await?;

    if !resp.success {
        return Err(DeployError::Api { errors: resp.errors });
    }

    Ok(())
}

async fn get_subdomain(
    client: &reqwest::Client,
    cf: &CfAccount,
) -> Result<String, DeployError> {
    let url = format!(
        "{CF_API_BASE}/accounts/{}/workers/subdomain",
        cf.account_id
    );

    let resp: CfResponse<SubdomainResult> = client
        .get(&url)
        .bearer_auth(&cf.api_token)
        .send()
        .await?
        .json()
        .await?;

    if !resp.success {
        return Err(DeployError::Api { errors: resp.errors });
    }

    resp.result
        .map(|r| r.subdomain)
        .ok_or(DeployError::NoSubdomain)
}
