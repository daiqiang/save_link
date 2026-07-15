//! 百度网盘 OAuth 授权码流程与本机 token 仓库。

use crate::baidu_store::BaiduAccessTokenProvider;
use crate::cloud_store::{CloudStoreError, CloudStoreResult};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use reqwest::blocking::Client;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_OAUTH_BASE_URL: &str = "https://openapi.baidu.com";
const DEFAULT_SCOPE: &str = "basic,netdisk";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_CALLBACK_REQUEST_BYTES: usize = 16 * 1024;

#[derive(Debug)]
pub enum BaiduOAuthError {
    Config(String),
    Callback(String),
    Provider(String),
    Network(String),
    Io(String),
}

impl fmt::Display for BaiduOAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "百度授权配置无效: {message}"),
            Self::Callback(message) => write!(f, "百度授权回调失败: {message}"),
            Self::Provider(message) => write!(f, "百度授权失败: {message}"),
            Self::Network(message) => write!(f, "无法连接百度授权服务: {message}"),
            Self::Io(message) => write!(f, "无法保存百度授权信息: {message}"),
        }
    }
}

impl std::error::Error for BaiduOAuthError {}

pub type BaiduOAuthResult<T> = std::result::Result<T, BaiduOAuthError>;

#[derive(Debug, Clone)]
pub struct BaiduOAuthConfig {
    app_key: String,
    secret_key: String,
    redirect_uri: String,
    scope: String,
    oauth_base_url: String,
    timeout: Duration,
}

impl BaiduOAuthConfig {
    pub fn new(
        app_key: impl Into<String>,
        secret_key: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> BaiduOAuthResult<Self> {
        Self::with_options(
            app_key,
            secret_key,
            redirect_uri,
            DEFAULT_SCOPE,
            DEFAULT_OAUTH_BASE_URL,
            DEFAULT_TIMEOUT,
        )
    }

    pub fn with_options(
        app_key: impl Into<String>,
        secret_key: impl Into<String>,
        redirect_uri: impl Into<String>,
        scope: impl Into<String>,
        oauth_base_url: impl Into<String>,
        timeout: Duration,
    ) -> BaiduOAuthResult<Self> {
        let config = Self {
            app_key: app_key.into(),
            secret_key: secret_key.into(),
            redirect_uri: redirect_uri.into(),
            scope: scope.into(),
            oauth_base_url: oauth_base_url.into().trim_end_matches('/').to_string(),
            timeout,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    fn validate(&self) -> BaiduOAuthResult<()> {
        if self.app_key.trim().is_empty() {
            return Err(BaiduOAuthError::Config("缺少 AppKey".into()));
        }
        if self.secret_key.trim().is_empty() {
            return Err(BaiduOAuthError::Config("缺少 SecretKey".into()));
        }
        if self.scope.trim().is_empty() {
            return Err(BaiduOAuthError::Config("scope 不能为空".into()));
        }
        if self.timeout.is_zero() {
            return Err(BaiduOAuthError::Config("请求超时必须大于 0".into()));
        }
        Url::parse(&self.oauth_base_url)
            .map_err(|_| BaiduOAuthError::Config("OAuth 服务地址不是有效 URL".into()))?;
        validate_loopback_redirect(&self.redirect_uri)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaiduOAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub scope: Option<String>,
    pub session_key: Option<String>,
    pub session_secret: Option<String>,
    pub saved_at: String,
    pub expires_at: Option<String>,
}

pub struct BaiduOAuthClient {
    config: BaiduOAuthConfig,
    client: Client,
}

impl BaiduOAuthClient {
    pub fn new(config: BaiduOAuthConfig) -> BaiduOAuthResult<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| BaiduOAuthError::Network(error.to_string()))?;
        Ok(Self { config, client })
    }

    pub fn authorization_url(&self, state: &str) -> BaiduOAuthResult<String> {
        if state.trim().is_empty() {
            return Err(BaiduOAuthError::Config("OAuth state 不能为空".into()));
        }
        let mut url = self.oauth_url("/oauth/2.0/authorize")?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.app_key)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("scope", &self.config.scope)
            .append_pair("state", state);
        Ok(url.to_string())
    }

    pub fn exchange_code(&self, code: &str) -> BaiduOAuthResult<BaiduOAuthToken> {
        if code.trim().is_empty() {
            return Err(BaiduOAuthError::Callback("授权码为空".into()));
        }
        self.request_token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", &self.config.app_key),
            ("client_secret", &self.config.secret_key),
            ("redirect_uri", &self.config.redirect_uri),
        ])
    }

    pub fn refresh_access_token(&self, refresh_token: &str) -> BaiduOAuthResult<BaiduOAuthToken> {
        if refresh_token.trim().is_empty() {
            return Err(BaiduOAuthError::Provider(
                "refresh_token 为空，需要重新授权".into(),
            ));
        }
        self.request_token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &self.config.app_key),
            ("client_secret", &self.config.secret_key),
        ])
    }

    fn oauth_url(&self, path: &str) -> BaiduOAuthResult<Url> {
        let base = Url::parse(&self.config.oauth_base_url)
            .map_err(|_| BaiduOAuthError::Config("OAuth 服务地址不是有效 URL".into()))?;
        base.join(path)
            .map_err(|_| BaiduOAuthError::Config("无法构造百度 OAuth URL".into()))
    }

    fn request_token(&self, parameters: &[(&str, &str)]) -> BaiduOAuthResult<BaiduOAuthToken> {
        let url = self.oauth_url("/oauth/2.0/token")?;
        let response = self
            .client
            .get(url)
            .query(parameters)
            .send()
            .map_err(|error| BaiduOAuthError::Network(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|error| BaiduOAuthError::Network(error.to_string()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| BaiduOAuthError::Provider("百度返回了无法解析的授权响应".into()))?;

        if let Some(error) = value.get("error").and_then(Value::as_str) {
            let description = value
                .get("error_description")
                .and_then(Value::as_str)
                .unwrap_or("");
            return Err(BaiduOAuthError::Provider(format!(
                "{error}{}",
                if description.is_empty() {
                    String::new()
                } else {
                    format!(": {description}")
                }
            )));
        }
        if !status.is_success() {
            return Err(BaiduOAuthError::Provider(format!("HTTP {status}")));
        }

        let access_token = value
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| BaiduOAuthError::Provider("响应中缺少 access_token".into()))?
            .to_string();
        let expires_in = value.get("expires_in").and_then(value_to_i64).unwrap_or(0);
        let saved = Utc::now();
        let expires_at = (expires_in > 0).then(|| {
            (saved + ChronoDuration::seconds(expires_in)).to_rfc3339_opts(SecondsFormat::Secs, true)
        });

        Ok(BaiduOAuthToken {
            access_token,
            refresh_token: optional_string(&value, "refresh_token"),
            expires_in,
            scope: optional_string(&value, "scope"),
            session_key: optional_string(&value, "session_key"),
            session_secret: optional_string(&value, "session_secret"),
            saved_at: saved.to_rfc3339_opts(SecondsFormat::Secs, true),
            expires_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FileBaiduTokenStore {
    path: PathBuf,
}

impl FileBaiduTokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> BaiduOAuthResult<Option<BaiduOAuthToken>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path).map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        let token = serde_json::from_slice(&bytes)
            .map_err(|_| BaiduOAuthError::Io("本机百度 token 文件已损坏".into()))?;
        Ok(Some(token))
    }

    pub fn save(&self, token: &BaiduOAuthToken) -> BaiduOAuthResult<()> {
        if token.access_token.trim().is_empty() {
            return Err(BaiduOAuthError::Io("不能保存空 access_token".into()));
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        }
        let bytes = serde_json::to_vec_pretty(token)
            .map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        let temp_path = self.path.with_extension("json.tmp");
        fs::write(&temp_path, bytes).map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        }
        fs::rename(&temp_path, &self.path)
            .map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        Ok(())
    }

    pub fn clear(&self) -> BaiduOAuthResult<()> {
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|error| BaiduOAuthError::Io(error.to_string()))?;
        }
        Ok(())
    }
}

impl BaiduAccessTokenProvider for FileBaiduTokenStore {
    fn access_token(&self) -> CloudStoreResult<String> {
        self.load()
            .map_err(|error| CloudStoreError::Provider(error.to_string()))?
            .map(|token| token.access_token)
            .filter(|token| !token.trim().is_empty())
            .ok_or(CloudStoreError::AuthRequired)
    }
}

pub struct OAuthCallbackListener {
    listener: TcpListener,
    callback_path: String,
}

impl OAuthCallbackListener {
    pub fn bind(redirect_uri: &str) -> BaiduOAuthResult<Self> {
        let redirect = validate_loopback_redirect(redirect_uri)?;
        let host = redirect
            .host_str()
            .ok_or_else(|| BaiduOAuthError::Config("回调地址缺少主机".into()))?;
        let port = redirect
            .port()
            .ok_or_else(|| BaiduOAuthError::Config("回调地址缺少端口".into()))?;
        let listener = TcpListener::bind((host, port)).map_err(|error| {
            BaiduOAuthError::Callback(format!("无法监听 {host}:{port}: {error}"))
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| BaiduOAuthError::Callback(error.to_string()))?;
        Ok(Self {
            listener,
            callback_path: redirect.path().to_string(),
        })
    }

    pub fn wait_for_code(
        self,
        expected_state: &str,
        timeout: Duration,
    ) -> BaiduOAuthResult<String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(result) = self.handle_request(&mut stream, expected_state)? {
                        return result;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(BaiduOAuthError::Callback(
                            "等待授权超时，请重新点击云上传按钮".into(),
                        ));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(BaiduOAuthError::Callback(error.to_string())),
            }
        }
    }

    fn handle_request(
        &self,
        stream: &mut TcpStream,
        expected_state: &str,
    ) -> BaiduOAuthResult<Option<BaiduOAuthResult<String>>> {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| BaiduOAuthError::Callback(error.to_string()))?;
        let request_bytes = read_callback_request(stream)?;
        let request = String::from_utf8_lossy(&request_bytes);
        let request_line = request.lines().next().unwrap_or_default();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default();
        let target = parts.next().unwrap_or_default();
        if method != "GET" || target.is_empty() {
            write_callback_response(stream, 400, "授权回调请求无效")?;
            return Ok(Some(Err(BaiduOAuthError::Callback(
                "浏览器回调请求无效".into(),
            ))));
        }

        let request_url = Url::parse(&format!("http://127.0.0.1{target}"))
            .map_err(|_| BaiduOAuthError::Callback("浏览器回调 URL 无效".into()))?;
        if request_url.path() != self.callback_path {
            write_callback_response(stream, 404, "Not found")?;
            return Ok(None);
        }

        let query: std::collections::HashMap<String, String> =
            request_url.query_pairs().into_owned().collect();
        if let Some(error) = query.get("error") {
            write_callback_response(stream, 400, "百度网盘授权未完成，可以关闭此页面")?;
            let description = query.get("error_description").cloned().unwrap_or_default();
            return Ok(Some(Err(BaiduOAuthError::Provider(format!(
                "{error}{}",
                if description.is_empty() {
                    String::new()
                } else {
                    format!(": {description}")
                }
            )))));
        }
        if query.get("state").map(String::as_str) != Some(expected_state) {
            write_callback_response(stream, 400, "授权校验失败，可以关闭此页面")?;
            return Ok(None);
        }
        let code = query
            .get("code")
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| BaiduOAuthError::Callback("回调中缺少授权码".into()))?;
        write_callback_response(stream, 200, "SaveLink 已成功连接百度网盘，可以关闭此页面")?;
        Ok(Some(Ok(code)))
    }
}

pub fn new_oauth_state() -> BaiduOAuthResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| BaiduOAuthError::Config(format!("无法生成安全随机数: {error}")))?;
    let mut state = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        write!(&mut state, "{byte:02x}")
            .map_err(|error| BaiduOAuthError::Config(error.to_string()))?;
    }
    Ok(state)
}

fn validate_loopback_redirect(redirect_uri: &str) -> BaiduOAuthResult<Url> {
    let url = Url::parse(redirect_uri)
        .map_err(|_| BaiduOAuthError::Config("回调地址不是有效 URL".into()))?;
    if url.scheme() != "http" {
        return Err(BaiduOAuthError::Config(
            "桌面端回调地址必须使用 http".into(),
        ));
    }
    if !matches!(url.host_str(), Some("127.0.0.1") | Some("localhost")) {
        return Err(BaiduOAuthError::Config(
            "桌面端回调地址必须指向 127.0.0.1 或 localhost".into(),
        ));
    }
    if url.port().is_none() {
        return Err(BaiduOAuthError::Config("回调地址必须包含端口".into()));
    }
    if url.path().is_empty() || url.path() == "/" {
        return Err(BaiduOAuthError::Config("回调地址必须包含路径".into()));
    }
    Ok(url)
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn value_to_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn read_callback_request(stream: &mut TcpStream) -> BaiduOAuthResult<Vec<u8>> {
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let count = stream
            .read(&mut chunk)
            .map_err(|error| BaiduOAuthError::Callback(error.to_string()))?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() >= MAX_CALLBACK_REQUEST_BYTES {
            return Err(BaiduOAuthError::Callback("浏览器回调请求过大".into()));
        }
    }
    if request.is_empty() {
        return Err(BaiduOAuthError::Callback("浏览器回调请求为空".into()));
    }
    Ok(request)
}

fn write_callback_response(
    stream: &mut TcpStream,
    status: u16,
    message: &str,
) -> BaiduOAuthResult<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>SaveLink</title><body style=\"font-family:system-ui;padding:48px;color:#1f2937\"><h2>{message}</h2><p>现在可以返回 SaveLink。</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| BaiduOAuthError::Callback(error.to_string()))?;
    stream
        .flush()
        .map_err(|error| BaiduOAuthError::Callback(error.to_string()))
}
