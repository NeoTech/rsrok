use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let worker_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("worker");
    let embedded_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("embedded");

    // Rerun if worker or protocol sources change
    println!("cargo:rerun-if-changed=../worker/src/");
    println!("cargo:rerun-if-changed=../worker-wasm/src/lib.rs");
    println!("cargo:rerun-if-changed=../protocol/src/lib.rs");

    let dist_dir = worker_dir.join("dist");
    let js_target = embedded_dir.join("worker.js");
    let wasm_target = embedded_dir.join("worker.wasm");
    let name_target = embedded_dir.join("wasm_module_name.txt");

    // If dist/ has been rebuilt (e.g. `bun run build:bundle` was run), prefer
    // the fresh artifacts over stale embedded ones.
    let dist_js = dist_dir.join("index.js");
    if dist_js.exists() {
        if !js_target.exists() || is_newer(&dist_js, &js_target) {
            copy_artifacts(&dist_dir, &js_target, &wasm_target, &name_target);
            let _ = fs::remove_dir_all(&dist_dir);
            return;
        }
    }

    // If artifacts already exist, use them as-is. We must NOT call
    // `bun run build:bundle` from inside build.rs when Cargo holds its lock,
    // because the bundle step runs wasm-pack which spawns `cargo build` for the
    // WASM target — that child cargo blocks waiting for the same lock → deadlock.
    //
    // Workflow:
    //   1. cd worker && bun run build:bundle   (one-time, or after worker changes)
    //   2. cargo build [--release]             (embeds existing artifacts)
    if js_target.exists() && wasm_target.exists() && name_target.exists() {
        return;
    }

    // No pre-built artifacts — try building. This only succeeds when no parent
    // cargo process holds the build lock (e.g. first-ever build from clean state
    // when running `cargo build` on just this crate).
    let build_ok = try_build_bundle(&worker_dir, &dist_dir);

    if build_ok {
        copy_artifacts(&dist_dir, &js_target, &wasm_target, &name_target);
        let _ = fs::remove_dir_all(&dist_dir);
    } else {
        panic!(
            "No pre-built worker artifacts in cli/src/embedded/.\n\
             Build them first:\n\n  \
             cd worker && bun run build:bundle\n\n\
             Then re-run cargo build."
        );
    }
}

fn try_build_bundle(worker_dir: &Path, dist_dir: &Path) -> bool {
    // Clean previous dist output
    let _ = fs::remove_dir_all(dist_dir);

    let result = Command::new("bun")
        .args(["run", "build:bundle"])
        .current_dir(worker_dir)
        .output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("cargo:warning=wrangler build failed: {stderr}");
                return false;
            }
            true
        }
        Err(e) => {
            println!("cargo:warning=could not run bun: {e}");
            false
        }
    }
}

fn copy_artifacts(
    dist_dir: &Path,
    js_target: &Path,
    wasm_target: &Path,
    name_target: &Path,
) {
    // Copy index.js
    let js_src = dist_dir.join("index.js");
    assert!(
        js_src.exists(),
        "wrangler build did not produce dist/index.js"
    );
    fs::copy(&js_src, js_target).expect("failed to copy index.js to embedded/");

    // Find the WASM file (hash-prefixed, e.g. {hash}-rs_rok_worker_wasm_bg.wasm)
    let wasm_src = fs::read_dir(dist_dir)
        .expect("cannot read dist/")
        .filter_map(|e| e.ok())
        .find(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "wasm")
        })
        .expect("wrangler build did not produce a .wasm file in dist/");

    let wasm_filename = wasm_src.file_name();
    let wasm_filename_str = wasm_filename.to_string_lossy();

    fs::copy(wasm_src.path(), wasm_target).expect("failed to copy .wasm to embedded/");

    // Write the WASM module name so worker_bundle.rs knows the import path
    // The JS bundle imports it as "./{wasm_filename}", so the multipart upload
    // must use "./{wasm_filename}" as the part/module name.
    fs::write(name_target, wasm_filename_str.as_bytes())
        .expect("failed to write wasm_module_name.txt");
}

fn is_newer(a: &Path, b: &Path) -> bool {
    let Ok(a_meta) = fs::metadata(a) else { return false };
    let Ok(b_meta) = fs::metadata(b) else { return true };
    let Ok(a_time) = a_meta.modified() else { return false };
    let Ok(b_time) = b_meta.modified() else { return true };
    a_time > b_time
}
