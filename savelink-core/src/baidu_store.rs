//! 百度网盘 `CloudObjectStore` 适配器。
//!
//! 本模块只负责对象存储语义和百度文件 API。OAuth 登录、token 刷新与凭据持久化
//! 由上层账号连接模块负责，并通过 `BaiduAccessTokenProvider` 提供当前 access token。

use crate::cloud_protocol::CLOUD_ROOT;
use crate::cloud_store::{
    CloudEntry, CloudEntryKind, CloudFile, CloudObjectStore, CloudStoreError, CloudStoreResult,
    PutMode,
};
use reqwest::blocking::multipart::{Form, Part};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_API_BASE_URL: &str = "https://pan.baidu.com";
const DEFAULT_UPLOAD_BASE_URL: &str = "https://d.pcs.baidu.com";
const DEFAULT_REMOTE_ROOT: &str = "/apps/savelink/v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_LIST_PAGE_SIZE: usize = 1000;
const SINGLE_STEP_UPLOAD_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const USER_AGENT: &str = "pan.baidu.com";

static DOWNLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 每次请求时提供当前 access token。实现方可以从内存凭据仓库读取最新值。
pub trait BaiduAccessTokenProvider: Send + Sync {
    fn access_token(&self) -> CloudStoreResult<String>;
}

/// 用于测试和人工接线实验的固定 token 提供者，不负责刷新。
pub struct StaticBaiduAccessToken {
    token: String,
}

impl StaticBaiduAccessToken {
    pub fn new(token: impl Into<String>) -> CloudStoreResult<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(CloudStoreError::AuthRequired);
        }
        Ok(Self { token })
    }
}

impl BaiduAccessTokenProvider for StaticBaiduAccessToken {
    fn access_token(&self) -> CloudStoreResult<String> {
        Ok(self.token.clone())
    }
}

#[derive(Debug, Clone)]
pub struct BaiduNetdiskConfig {
    pub api_base_url: String,
    pub upload_base_url: String,
    pub logical_root: String,
    pub remote_root: String,
    pub timeout: Duration,
    pub list_page_size: usize,
}

impl Default for BaiduNetdiskConfig {
    fn default() -> Self {
        Self {
            api_base_url: DEFAULT_API_BASE_URL.into(),
            upload_base_url: DEFAULT_UPLOAD_BASE_URL.into(),
            logical_root: CLOUD_ROOT.into(),
            remote_root: DEFAULT_REMOTE_ROOT.into(),
            timeout: DEFAULT_TIMEOUT,
            list_page_size: MAX_LIST_PAGE_SIZE,
        }
    }
}

pub struct BaiduNetdiskStore {
    client: Client,
    config: BaiduNetdiskConfig,
    token_provider: Arc<dyn BaiduAccessTokenProvider>,
    ensured_directories: Mutex<HashSet<String>>,
}

impl BaiduNetdiskStore {
    pub fn new(token_provider: Arc<dyn BaiduAccessTokenProvider>) -> CloudStoreResult<Self> {
        Self::with_config(token_provider, BaiduNetdiskConfig::default())
    }

    pub fn with_config(
        token_provider: Arc<dyn BaiduAccessTokenProvider>,
        config: BaiduNetdiskConfig,
    ) -> CloudStoreResult<Self> {
        validate_config(&config)?;
        let client = Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|_| CloudStoreError::Provider("无法初始化百度网盘 HTTP 客户端".into()))?;
        Ok(Self {
            client,
            config,
            token_provider,
            ensured_directories: Mutex::new(HashSet::new()),
        })
    }

    fn access_token(&self) -> CloudStoreResult<String> {
        let token = self.token_provider.access_token()?;
        if token.trim().is_empty() {
            return Err(CloudStoreError::AuthRequired);
        }
        Ok(token)
    }

    fn api_url(&self, path: &str) -> CloudStoreResult<Url> {
        join_url(&self.config.api_base_url, path)
    }

    fn upload_url(&self, path: &str) -> CloudStoreResult<Url> {
        join_url(&self.config.upload_base_url, path)
    }

    fn remote_path(&self, logical_path: &str, allow_root: bool) -> CloudStoreResult<String> {
        if logical_path.is_empty() {
            return if allow_root {
                Ok(self.config.remote_root.clone())
            } else {
                Err(CloudStoreError::InvalidPath(logical_path.into()))
            };
        }
        validate_logical_path(logical_path)?;
        if logical_path == self.config.logical_root {
            return Ok(self.config.remote_root.clone());
        }
        let prefix = format!("{}/", self.config.logical_root);
        let suffix = logical_path
            .strip_prefix(&prefix)
            .ok_or_else(|| CloudStoreError::InvalidPath(logical_path.into()))?;
        Ok(format!("{}/{}", self.config.remote_root, suffix))
    }

    fn logical_child(&self, parent: &str, name: &str) -> CloudStoreResult<String> {
        validate_path_segment(name, name)?;
        let parent = if parent.is_empty() {
            self.config.logical_root.as_str()
        } else {
            parent
        };
        Ok(format!("{parent}/{name}"))
    }

    fn send(&self, request: RequestBuilder) -> CloudStoreResult<Response> {
        request.send().map_err(map_request_error)
    }

    fn read_json_response(
        &self,
        response: Response,
        logical_path: &str,
        allow_already_exists: bool,
    ) -> CloudStoreResult<Value> {
        let status = response.status();
        if !status.is_success() {
            return Err(map_http_status(status, logical_path));
        }
        let bytes = response.bytes().map_err(map_request_error)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| CloudStoreError::Provider("百度网盘返回了无法解析的 JSON 响应".into()))?;
        check_baidu_error(&value, logical_path, allow_already_exists)?;
        Ok(value)
    }

    fn create_directory(&self, remote_dir: &str) -> CloudStoreResult<()> {
        let token = self.access_token()?;
        let request = self
            .client
            .post(self.api_url("/rest/2.0/xpan/file")?)
            .query(&[("method", "create"), ("access_token", token.as_str())])
            .form(&[("path", remote_dir), ("isdir", "1"), ("rtype", "0")]);
        let response = self.send(request)?;
        self.read_json_response(response, remote_dir, true)?;
        Ok(())
    }

    fn ensure_parent_directories(&self, remote_file: &str) -> CloudStoreResult<()> {
        let parent = posix_dirname(remote_file);
        let mut current = String::new();
        for segment in parent.split('/').filter(|segment| !segment.is_empty()) {
            current.push('/');
            current.push_str(segment);
            if current == "/apps" {
                continue;
            }
            if self
                .ensured_directories
                .lock()
                .map_err(|_| CloudStoreError::Provider("百度网盘目录缓存不可用".into()))?
                .contains(&current)
            {
                continue;
            }
            self.create_directory(&current)?;
            self.ensured_directories
                .lock()
                .map_err(|_| CloudStoreError::Provider("百度网盘目录缓存不可用".into()))?
                .insert(current.clone());
        }
        Ok(())
    }

    fn list_remote_directory(&self, remote_dir: &str) -> CloudStoreResult<Vec<BaiduEntry>> {
        let mut entries = Vec::new();
        let mut start = 0usize;
        loop {
            let token = self.access_token()?;
            let start_text = start.to_string();
            let limit_text = self.config.list_page_size.to_string();
            let request = self
                .client
                .get(self.api_url("/rest/2.0/xpan/file")?)
                .header("User-Agent", USER_AGENT)
                .query(&[
                    ("method", "list"),
                    ("access_token", token.as_str()),
                    ("dir", remote_dir),
                    ("order", "name"),
                    ("start", start_text.as_str()),
                    ("limit", limit_text.as_str()),
                    ("web", "1"),
                    ("folder", "0"),
                    ("desc", "0"),
                ]);
            let response = self.send(request)?;
            let value = self.read_json_response(response, remote_dir, false)?;
            let page: BaiduListResponse = serde_json::from_value(value).map_err(|_| {
                CloudStoreError::Provider("百度网盘文件列表响应缺少必要字段".into())
            })?;
            let count = page.list.len();
            entries.extend(page.list);
            if count < self.config.list_page_size {
                break;
            }
            start = start.saturating_add(count);
        }
        Ok(entries)
    }

    fn find_entry(&self, logical_path: &str) -> CloudStoreResult<Option<BaiduEntry>> {
        let remote_file = self.remote_path(logical_path, false)?;
        let remote_parent = posix_dirname(&remote_file);
        let file_name = posix_basename(&remote_file);
        let entries = match self.list_remote_directory(remote_parent) {
            Ok(entries) => entries,
            Err(CloudStoreError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(entries.into_iter().find(|entry| {
            entry.path == remote_file || entry.server_filename.as_deref() == Some(file_name)
        }))
    }

    fn file_download_link(&self, entry: &BaiduEntry) -> CloudStoreResult<String> {
        let token = self.access_token()?;
        let fsids = serde_json::to_string(&[entry.fs_id])
            .map_err(|_| CloudStoreError::Provider("无法构造百度网盘文件信息请求".into()))?;
        let request = self
            .client
            .get(self.api_url("/rest/2.0/xpan/multimedia")?)
            .header("User-Agent", USER_AGENT)
            .query(&[
                ("method", "filemetas"),
                ("access_token", token.as_str()),
                ("fsids", fsids.as_str()),
                ("dlink", "1"),
            ]);
        let response = self.send(request)?;
        let value = self.read_json_response(response, &entry.path, false)?;
        let metas: BaiduFileMetasResponse = serde_json::from_value(value)
            .map_err(|_| CloudStoreError::Provider("百度网盘文件信息响应缺少必要字段".into()))?;
        metas
            .list
            .into_iter()
            .find(|meta| meta.fs_id == entry.fs_id)
            .and_then(|meta| meta.dlink)
            .ok_or_else(|| CloudStoreError::Provider("百度网盘未返回文件下载地址".into()))
    }

    fn download_to_temp(
        &self,
        logical_path: &str,
        entry: &BaiduEntry,
        local_file: &Path,
    ) -> CloudStoreResult<()> {
        let dlink = self.file_download_link(entry)?;
        let mut download_url = Url::parse(&dlink)
            .map_err(|_| CloudStoreError::Provider("百度网盘返回了无效的文件下载地址".into()))?;
        let token = self.access_token()?;
        download_url
            .query_pairs_mut()
            .append_pair("access_token", &token);

        let response = self.send(
            self.client
                .get(download_url)
                .header("User-Agent", USER_AGENT),
        )?;
        if !response.status().is_success() {
            return Err(map_http_status(response.status(), logical_path));
        }
        if let Some(parent) = local_file.parent() {
            fs::create_dir_all(parent).map_err(|error| CloudStoreError::Io(error.to_string()))?;
        }
        let temp_file = download_temp_path(local_file);
        let result = (|| {
            let mut output =
                File::create(&temp_file).map_err(|error| CloudStoreError::Io(error.to_string()))?;
            let mut response = response;
            let written = response.copy_to(&mut output).map_err(map_request_error)?;
            output
                .flush()
                .map_err(|error| CloudStoreError::Io(error.to_string()))?;
            if written != entry.size {
                return Err(CloudStoreError::Provider(format!(
                    "百度网盘下载大小不一致: 期望 {} 字节，实际 {} 字节",
                    entry.size, written
                )));
            }
            if local_file.exists() {
                fs::remove_file(local_file)
                    .map_err(|error| CloudStoreError::Io(error.to_string()))?;
            }
            fs::rename(&temp_file, local_file)
                .map_err(|error| CloudStoreError::Io(error.to_string()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_file);
        }
        result
    }
}

impl CloudObjectStore for BaiduNetdiskStore {
    fn put_file(
        &self,
        remote_path: &str,
        local_file: &Path,
        mode: PutMode,
    ) -> CloudStoreResult<CloudFile> {
        let metadata = fs::metadata(local_file)
            .map_err(|error| CloudStoreError::Io(format!("无法读取本机上传文件: {error}")))?;
        if !metadata.is_file() {
            return Err(CloudStoreError::Io(format!(
                "本机上传源不是文件: {}",
                local_file.display()
            )));
        }
        if metadata.len() > SINGLE_STEP_UPLOAD_LIMIT {
            return Err(CloudStoreError::Provider(
                "百度网盘单步上传暂不支持超过 2 GiB 的对象".into(),
            ));
        }
        let physical_path = self.remote_path(remote_path, false)?;
        if mode == PutMode::CreateOnly && self.find_entry(remote_path)?.is_some() {
            return Err(CloudStoreError::AlreadyExists(remote_path.into()));
        }
        self.ensure_parent_directories(&physical_path)?;

        let token = self.access_token()?;
        let file_name = posix_basename(&physical_path).to_string();
        let file =
            File::open(local_file).map_err(|error| CloudStoreError::Io(error.to_string()))?;
        let part = Part::reader_with_length(file, metadata.len()).file_name(file_name);
        let form = Form::new().part("file", part);
        let ondup = match mode {
            PutMode::CreateOnly => "fail",
            PutMode::Overwrite => "overwrite",
        };
        let request = self
            .client
            .post(self.upload_url("/rest/2.0/pcs/file")?)
            .header("User-Agent", USER_AGENT)
            .query(&[
                ("method", "upload"),
                ("access_token", token.as_str()),
                ("path", physical_path.as_str()),
                ("ondup", ondup),
            ])
            .multipart(form);
        let response = self.send(request)?;
        self.read_json_response(response, remote_path, false)?;
        Ok(CloudFile {
            path: remote_path.into(),
            size: metadata.len(),
        })
    }

    fn get_file(&self, remote_path: &str, local_file: &Path) -> CloudStoreResult<()> {
        let entry = self
            .find_entry(remote_path)?
            .ok_or_else(|| CloudStoreError::NotFound(remote_path.into()))?;
        if entry.isdir == 1 {
            return Err(CloudStoreError::InvalidPath(remote_path.into()));
        }
        self.download_to_temp(remote_path, &entry, local_file)
    }

    fn list_directory(&self, remote_path: &str) -> CloudStoreResult<Vec<CloudEntry>> {
        let physical_path = self.remote_path(remote_path, true)?;
        let entries = self.list_remote_directory(&physical_path)?;
        let mut result = Vec::with_capacity(entries.len());
        for entry in entries {
            let name = entry
                .server_filename
                .unwrap_or_else(|| posix_basename(&entry.path).to_string());
            let kind = if entry.isdir == 1 {
                CloudEntryKind::Directory
            } else {
                CloudEntryKind::File
            };
            result.push(CloudEntry {
                path: self.logical_child(remote_path, &name)?,
                name,
                kind,
                size: if kind == CloudEntryKind::File {
                    entry.size
                } else {
                    0
                },
            });
        }
        result.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(result)
    }

    fn stat_file(&self, remote_path: &str) -> CloudStoreResult<Option<CloudFile>> {
        let Some(entry) = self.find_entry(remote_path)? else {
            return Ok(None);
        };
        if entry.isdir == 1 {
            return Err(CloudStoreError::InvalidPath(remote_path.into()));
        }
        Ok(Some(CloudFile {
            path: remote_path.into(),
            size: entry.size,
        }))
    }

    fn delete_file(&self, remote_path: &str) -> CloudStoreResult<()> {
        let Some(entry) = self.find_entry(remote_path)? else {
            return Ok(());
        };
        if entry.isdir == 1 {
            return Err(CloudStoreError::InvalidPath(remote_path.into()));
        }
        let token = self.access_token()?;
        let file_list = serde_json::to_string(&[serde_json::json!({ "path": entry.path })])
            .map_err(|_| CloudStoreError::Provider("无法构造百度网盘删除请求".into()))?;
        let request = self
            .client
            .post(self.api_url("/rest/2.0/xpan/file")?)
            .query(&[
                ("method", "filemanager"),
                ("access_token", token.as_str()),
                ("opera", "delete"),
            ])
            .form(&[("async", "0"), ("filelist", file_list.as_str())]);
        let response = self.send(request)?;
        let value = match self.read_json_response(response, remote_path, false) {
            Err(CloudStoreError::NotFound(_)) => return Ok(()),
            other => other?,
        };
        if let Some(info) = value.get("info").and_then(Value::as_array) {
            for item in info {
                if let Err(error) = check_baidu_error(item, remote_path, false) {
                    if !matches!(error, CloudStoreError::NotFound(_)) {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct BaiduListResponse {
    #[serde(default)]
    list: Vec<BaiduEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct BaiduEntry {
    fs_id: u64,
    path: String,
    #[serde(default)]
    server_filename: Option<String>,
    #[serde(default)]
    isdir: i32,
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Deserialize)]
struct BaiduFileMetasResponse {
    #[serde(default)]
    list: Vec<BaiduFileMeta>,
}

#[derive(Debug, Deserialize)]
struct BaiduFileMeta {
    fs_id: u64,
    dlink: Option<String>,
}

fn validate_config(config: &BaiduNetdiskConfig) -> CloudStoreResult<()> {
    validate_base_url(&config.api_base_url)?;
    validate_base_url(&config.upload_base_url)?;
    validate_logical_path(&config.logical_root)?;
    if !config.remote_root.starts_with('/')
        || config.remote_root.ends_with('/')
        || config.remote_root.contains('\\')
        || config.remote_root.contains('\0')
    {
        return Err(CloudStoreError::InvalidPath(config.remote_root.clone()));
    }
    for segment in config
        .remote_root
        .split('/')
        .filter(|value| !value.is_empty())
    {
        validate_path_segment(segment, &config.remote_root)?;
    }
    if config.list_page_size == 0 || config.list_page_size > MAX_LIST_PAGE_SIZE {
        return Err(CloudStoreError::Provider(
            "百度网盘列表分页大小必须在 1 到 1000 之间".into(),
        ));
    }
    Ok(())
}

fn validate_base_url(value: &str) -> CloudStoreResult<()> {
    let url =
        Url::parse(value).map_err(|_| CloudStoreError::Provider("百度网盘 API 地址无效".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(CloudStoreError::Provider(
            "百度网盘 API 地址必须是 HTTP(S) URL".into(),
        ));
    }
    Ok(())
}

fn validate_logical_path(path: &str) -> CloudStoreResult<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains('\0')
    {
        return Err(CloudStoreError::InvalidPath(path.into()));
    }
    for segment in path.split('/') {
        validate_path_segment(segment, path)?;
    }
    Ok(())
}

fn validate_path_segment(segment: &str, original: &str) -> CloudStoreResult<()> {
    if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') {
        return Err(CloudStoreError::InvalidPath(original.into()));
    }
    Ok(())
}

fn join_url(base: &str, path: &str) -> CloudStoreResult<Url> {
    let base =
        Url::parse(base).map_err(|_| CloudStoreError::Provider("百度网盘 API 地址无效".into()))?;
    base.join(path)
        .map_err(|_| CloudStoreError::Provider("无法构造百度网盘 API 地址".into()))
}

fn posix_dirname(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

fn posix_basename(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path)
}

fn download_temp_path(target: &Path) -> PathBuf {
    let sequence = DOWNLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".into());
    target.with_file_name(format!(
        ".{name}.savelink-download-{}-{sequence}.part",
        std::process::id()
    ))
}

fn map_request_error(error: reqwest::Error) -> CloudStoreError {
    if error.is_timeout() || error.is_connect() || error.is_body() {
        CloudStoreError::NetworkUnavailable
    } else {
        CloudStoreError::Provider("百度网盘 HTTP 请求失败".into())
    }
}

fn map_http_status(status: StatusCode, logical_path: &str) -> CloudStoreError {
    match status.as_u16() {
        401 | 403 => CloudStoreError::AuthRequired,
        404 => CloudStoreError::NotFound(logical_path.into()),
        408 => CloudStoreError::NetworkUnavailable,
        429 => CloudStoreError::RateLimited,
        _ => CloudStoreError::Provider(format!("百度网盘 HTTP 状态码 {}", status.as_u16())),
    }
}

fn check_baidu_error(
    value: &Value,
    logical_path: &str,
    allow_already_exists: bool,
) -> CloudStoreResult<()> {
    let code = value
        .get("errno")
        .and_then(Value::as_i64)
        .or_else(|| value.get("error_code").and_then(Value::as_i64));
    let Some(code) = code else {
        if value.get("error").and_then(Value::as_str).is_some() {
            return Err(CloudStoreError::AuthRequired);
        }
        return Ok(());
    };
    if code == 0 || (allow_already_exists && is_already_exists_code(code)) {
        return Ok(());
    }
    Err(match code {
        110 | 111 => CloudStoreError::AuthRequired,
        31034 | 31326 => CloudStoreError::RateLimited,
        -9 | 31066 => CloudStoreError::NotFound(logical_path.into()),
        value if is_already_exists_code(value) => {
            CloudStoreError::AlreadyExists(logical_path.into())
        }
        _ => CloudStoreError::Provider(format!("百度网盘 API 错误码 {code}")),
    })
}

fn is_already_exists_code(code: i64) -> bool {
    matches!(code, -8 | 31062)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_protocol_paths_to_baidu_application_directory() {
        let provider = Arc::new(StaticBaiduAccessToken::new("token").unwrap());
        let store = BaiduNetdiskStore::new(provider).unwrap();
        assert_eq!(
            store
                .remote_path("savelink/v1/games/game_1/game.json", false)
                .unwrap(),
            "/apps/savelink/v1/games/game_1/game.json"
        );
        assert!(matches!(
            store.remote_path("other/v1/file", false),
            Err(CloudStoreError::InvalidPath(_))
        ));
    }

    #[test]
    fn maps_stable_baidu_error_categories_without_response_secrets() {
        assert!(matches!(
            check_baidu_error(&serde_json::json!({ "errno": 111 }), "file", false),
            Err(CloudStoreError::AuthRequired)
        ));
        assert!(matches!(
            check_baidu_error(&serde_json::json!({ "errno": 31034 }), "file", false),
            Err(CloudStoreError::RateLimited)
        ));
        assert!(matches!(
            check_baidu_error(&serde_json::json!({ "errno": -9 }), "file", false),
            Err(CloudStoreError::NotFound(path)) if path == "file"
        ));
        assert!(matches!(
            check_baidu_error(&serde_json::json!({ "errno": -8 }), "file", false),
            Err(CloudStoreError::AlreadyExists(path)) if path == "file"
        ));
    }
}
