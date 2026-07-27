use serde_json::Value;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-env-changed=SAVELINK_BAIDU_APP_KEY");
    println!("cargo:rerun-if-env-changed=SAVELINK_BAIDU_SECRET_KEY");
    println!("cargo:rerun-if-env-changed=SAVELINK_BAIDU_REDIRECT_URI");
    println!("cargo:rerun-if-env-changed=SAVELINK_BAIDU_SCOPE");
    generate_baidu_oauth_config();
    tauri_build::build()
}

fn generate_baidu_oauth_config() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is required"));
    let local_path = manifest_dir.join(Path::new("baidu-oauth.local.json"));
    println!("cargo:rerun-if-changed={}", local_path.display());
    let local = read_local_config(&local_path);
    let app_key = config_value(&local, "SAVELINK_BAIDU_APP_KEY", "appKey", "");
    let secret_key = config_value(&local, "SAVELINK_BAIDU_SECRET_KEY", "secretKey", "");
    let redirect_uri = config_value(
        &local,
        "SAVELINK_BAIDU_REDIRECT_URI",
        "redirectUri",
        "http://127.0.0.1:53682/oauth/callback",
    );
    let scope = config_value(&local, "SAVELINK_BAIDU_SCOPE", "scope", "basic,netdisk");
    println!(
        "cargo:warning=Baidu OAuth build config: local_file={}, app_key_length={}, secret_key_length={}, redirect_uri_configured={}",
        local_path.exists(),
        app_key.len(),
        secret_key.len(),
        !redirect_uri.trim().is_empty()
    );
    let generated = format!(
        "pub const BAIDU_APP_KEY: &str = {app_key:?};\n\
         pub const BAIDU_SECRET_KEY: &str = {secret_key:?};\n\
         pub const BAIDU_REDIRECT_URI: &str = {redirect_uri:?};\n\
         pub const BAIDU_SCOPE: &str = {scope:?};\n"
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is required"));
    fs::write(out_dir.join("baidu_oauth_config.rs"), generated)
        .expect("failed to generate Baidu OAuth build config");
}

fn read_local_config(path: &Path) -> Value {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Value::Null,
        Err(error) => panic!(
            "failed to read Baidu OAuth config {}: {error}",
            path.display()
        ),
    };
    let json_bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice(json_bytes).unwrap_or_else(|error| {
        panic!(
            "failed to parse Baidu OAuth config {}: {error}",
            path.display()
        )
    })
}

fn config_value(local: &Value, env_key: &str, json_key: &str, default: &str) -> String {
    env::var(env_key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            local
                .get(json_key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| default.to_string())
}
