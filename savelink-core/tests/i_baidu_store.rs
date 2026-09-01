//! I 组：百度网盘 CloudObjectStore 正式适配器的本地 HTTP 契约测试。

use reqwest::Url;
use savelink_core::baidu_store::{BaiduNetdiskConfig, BaiduNetdiskStore, StaticBaiduAccessToken};
use savelink_core::cloud_store::{CloudEntryKind, CloudObjectStore, CloudStoreError, PutMode};
use savelink_core::testkit::TempDir;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct MockResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl MockResponse {
    fn json(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.as_bytes().to_vec(),
        }
    }

    fn bytes(body: &[u8]) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body: body.to_vec(),
        }
    }
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    request_line: String,
    headers: String,
    body: Vec<u8>,
}

struct ScriptedServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: Option<JoinHandle<()>>,
}

impl ScriptedServer {
    fn start(mut responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let base_url = format!("http://{address}");
        for response in &mut responses {
            let text = String::from_utf8_lossy(&response.body);
            if text.contains("{{BASE_URL}}") {
                response.body = text.replace("{{BASE_URL}}", &base_url).into_bytes();
            }
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = requests.clone();
        let expected_count = responses.len();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            for response in responses {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            break stream;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if Instant::now() >= deadline {
                                panic!("mock server timed out after receiving fewer than {expected_count} requests");
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("mock server accept failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_request(&mut stream);
                thread_requests.lock().unwrap().push(request);
                write_response(&mut stream, response);
            }
        });

        Self {
            base_url,
            requests,
            handle: Some(handle),
        }
    }

    fn store(&self) -> BaiduNetdiskStore {
        let config = BaiduNetdiskConfig {
            api_base_url: self.base_url.clone(),
            upload_base_url: self.base_url.clone(),
            timeout: Duration::from_secs(5),
            ..BaiduNetdiskConfig::default()
        };
        BaiduNetdiskStore::with_config(
            Arc::new(StaticBaiduAccessToken::new("test-access-token").unwrap()),
            config,
        )
        .unwrap()
    }

    fn finish(mut self) -> Vec<RecordedRequest> {
        self.handle.take().unwrap().join().unwrap();
        Arc::try_unwrap(self.requests)
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

#[test]
fn i1_create_only_upload_maps_path_and_uses_fail_mode() {
    let server = ScriptedServer::start(vec![
        MockResponse::json(r#"{"errno":0,"list":[]}"#),
        MockResponse::json(r#"{"errno":-8}"#),
        MockResponse::json(r#"{"errno":-8}"#),
        MockResponse::json(r#"{"errno":0,"path":"/apps/savelink/v1/manifest.json","size":10}"#),
    ]);
    let store = server.store();
    let tmp = TempDir::new();
    let source = tmp.path().join("manifest.json");
    fs::write(&source, b"save-bytes").unwrap();

    let uploaded = store
        .put_file("savelink/v1/manifest.json", &source, PutMode::CreateOnly)
        .unwrap();
    assert_eq!(uploaded.path, "savelink/v1/manifest.json");
    assert_eq!(uploaded.size, 10);

    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    let stat_query = request_query(&requests[0]);
    assert_eq!(query_value(&stat_query, "method"), Some("list"));
    assert_eq!(query_value(&stat_query, "dir"), Some("/apps/savelink/v1"));
    let upload_query = request_query(&requests[3]);
    assert_eq!(query_value(&upload_query, "method"), Some("upload"));
    assert_eq!(query_value(&upload_query, "ondup"), Some("fail"));
    assert_eq!(
        query_value(&upload_query, "path"),
        Some("/apps/savelink/v1/manifest.json")
    );
    assert!(requests[3]
        .body
        .windows(b"save-bytes".len())
        .any(|window| window == b"save-bytes"));
    assert!(requests[3]
        .headers
        .to_ascii_lowercase()
        .contains("content-type: multipart/form-data"));
}

#[test]
fn i2_create_only_stops_before_upload_when_remote_file_exists() {
    let server = ScriptedServer::start(vec![MockResponse::json(
        r#"{"errno":0,"list":[{"fs_id":7,"path":"/apps/savelink/v1/manifest.json","server_filename":"manifest.json","isdir":0,"size":10}]}"#,
    )]);
    let store = server.store();
    let tmp = TempDir::new();
    let source = tmp.path().join("manifest.json");
    fs::write(&source, b"save-bytes").unwrap();

    assert!(matches!(
        store.put_file("savelink/v1/manifest.json", &source, PutMode::CreateOnly),
        Err(CloudStoreError::AlreadyExists(path)) if path == "savelink/v1/manifest.json"
    ));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn i3_list_stat_download_and_delete_follow_baidu_file_api_contract() {
    let server = ScriptedServer::start(vec![
        MockResponse::json(
            r#"{"errno":0,"list":[{"fs_id":7,"path":"/apps/savelink/v1/games/game_1/snapshots/snap_1.ok","server_filename":"snap_1.ok","isdir":0,"size":9,"server_mtime":123},{"fs_id":8,"path":"/apps/savelink/v1/games/game_1/snapshots/archive","server_filename":"archive","isdir":1,"size":0}]}"#,
        ),
        MockResponse::json(r#"{"errno":0,"list":[{"fs_id":7,"dlink":"{{BASE_URL}}/download/7"}]}"#),
        MockResponse::bytes(b"cloud-ok!"),
        MockResponse::json(r#"{"errno":0,"info":[{"errno":0}]}"#),
        MockResponse::json(r#"{"errno":0,"list":[]}"#),
    ]);
    let store = server.store();
    let directory = "savelink/v1/games/game_1/snapshots";
    let remote_file = format!("{directory}/snap_1.ok");

    let entries = store.list_directory(directory).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "archive");
    assert_eq!(entries[0].kind, CloudEntryKind::Directory);
    assert_eq!(entries[1].name, "snap_1.ok");
    assert_eq!(entries[1].kind, CloudEntryKind::File);
    assert_eq!(entries[1].size, 9);
    assert_eq!(entries[1].modified_at, Some(123));
    assert_eq!(store.stat_file(&remote_file).unwrap().unwrap().size, 9);

    let tmp = TempDir::new();
    let downloaded = tmp.path().join("snap_1.ok");
    store.get_file(&remote_file, &downloaded).unwrap();
    assert_eq!(fs::read(&downloaded).unwrap(), b"cloud-ok!");
    assert_eq!(
        fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1
    );

    store.delete_file(&remote_file).unwrap();
    store.delete_file(&remote_file).unwrap();
    let requests = server.finish();
    assert_eq!(requests.len(), 5);
    let metas_query = request_query(&requests[1]);
    assert_eq!(query_value(&metas_query, "method"), Some("filemetas"));
    let download_query = request_query(&requests[2]);
    assert_eq!(
        query_value(&download_query, "access_token"),
        Some("test-access-token")
    );
    let delete_query = request_query(&requests[3]);
    assert_eq!(query_value(&delete_query, "method"), Some("filemanager"));
    assert_eq!(query_value(&delete_query, "opera"), Some("delete"));
    assert!(String::from_utf8_lossy(&requests[3].body).contains("filelist="));
}

#[test]
fn i4_missing_remote_parent_is_reported_as_absent_file() {
    let server = ScriptedServer::start(vec![MockResponse::json(r#"{"errno":-9}"#)]);
    let store = server.store();
    assert_eq!(
        store
            .stat_file("savelink/v1/games/game_1/game.json")
            .unwrap(),
        None
    );
    assert_eq!(server.finish().len(), 1);
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let (header_end, content_length) = loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            panic!("mock request ended before headers were complete");
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            break (header_end, content_length);
        }
    };
    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let request_line = headers.lines().next().unwrap().to_string();
    RecordedRequest {
        request_line,
        headers,
        body: bytes[body_start..].to_vec(),
    }
}

fn write_response(stream: &mut TcpStream, response: MockResponse) {
    let reason = if response.status == 200 {
        "OK"
    } else {
        "Error"
    };
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    stream.write_all(headers.as_bytes()).unwrap();
    stream.write_all(&response.body).unwrap();
    stream.flush().unwrap();
}

fn request_query(request: &RecordedRequest) -> Vec<(String, String)> {
    let target = request.request_line.split_whitespace().nth(1).unwrap();
    Url::parse(&format!("http://localhost{target}"))
        .unwrap()
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn query_value<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
