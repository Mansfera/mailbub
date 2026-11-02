use std::path::Path;

/// OAuth client ids/secrets may live in an untracked `.env.local` at the repo root; they are
/// compiled in via `option_env!`. Environment variables set at build time win.
fn load_env_local() {
    let path = Path::new("../.env.local");
    println!("cargo:rerun-if-changed=../.env.local");
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().trim_start_matches("export ").trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if key.starts_with("MAILBUB_") && std::env::var_os(key).is_none() {
                println!("cargo:rustc-env={key}={value}");
            }
        }
    }
}

fn main() {
    load_env_local();
    tauri_build::build()
}
