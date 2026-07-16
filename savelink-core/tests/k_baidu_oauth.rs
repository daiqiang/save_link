use savelink_core::baidu_oauth::{
    new_oauth_state, BaiduOAuthClient, BaiduOAuthConfig, BaiduOAuthToken, FileBaiduTokenStore,
    OAuthCallbackListener, RefreshingBaiduTokenProvider,
};
use savelink_core::baidu_store::BaiduAccessTokenProvider;
use savelink_core::testkit::TempDir;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn k01_authorization_url_contains_required_parameters() {
    let config = BaiduOAuthConfig::new(
        "app-key",
        "secret-key",
        "http://127.0.0.1:53682/oauth/callback",
    )
    .unwrap();
    let client = BaiduOAuthClient::new(config).unwrap();
    let url = reqwest::Url::parse(&client.authorization_url("state-123").unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(query.get("client_id").map(String::as_str), Some("app-key"));
    assert_eq!(
        query.get("redirect_uri").map(String::as_str),
        Some("http://127.0.0.1:53682/oauth/callback")
    );
    assert_eq!(
        query.get("scope").map(String::as_str),
        Some("basic,netdisk")
    );
    assert_eq!(query.get("state").map(String::as_str), Some("state-123"));
}

#[test]
fn k02_exchange_code_accepts_baidu_token_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 8192];
        let count = stream.read(&mut request).unwrap();
        request_tx
            .send(String::from_utf8_lossy(&request[..count]).to_string())
            .unwrap();
        let body = r#"{"access_token":"access-1","refresh_token":"refresh-1","expires_in":2592000,"scope":"basic netdisk"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let config = BaiduOAuthConfig::with_options(
        "app-key",
        "secret-key",
        "http://127.0.0.1:53682/oauth/callback",
        "basic,netdisk",
        format!("http://{address}"),
        Duration::from_secs(5),
    )
    .unwrap();
    let token = BaiduOAuthClient::new(config)
        .unwrap()
        .exchange_code("code-1")
        .unwrap();
    assert_eq!(token.access_token, "access-1");
    assert_eq!(token.refresh_token.as_deref(), Some("refresh-1"));
    assert_eq!(token.expires_in, 2_592_000);
    assert!(token.expires_at.is_some());

    let request = request_rx.recv().unwrap();
    assert!(request.contains("grant_type=authorization_code"));
    assert!(request.contains("code=code-1"));
    assert!(request.contains("client_id=app-key"));
    assert!(request.contains("client_secret=secret-key"));
}

#[test]
fn k03_token_store_round_trips_without_database_dependency() {
    let temp = TempDir::new();
    let store = FileBaiduTokenStore::new(temp.path().join("credentials/baidu-oauth.json"));
    let token = BaiduOAuthToken {
        access_token: "access-1".into(),
        refresh_token: Some("refresh-1".into()),
        expires_in: 3600,
        scope: Some("basic netdisk".into()),
        session_key: None,
        session_secret: None,
        saved_at: "2026-07-15T10:00:00Z".into(),
        expires_at: Some("2026-07-15T11:00:00Z".into()),
    };
    assert!(store.load().unwrap().is_none());
    store.save(&token).unwrap();
    assert_eq!(store.load().unwrap(), Some(token));
    store.clear().unwrap();
    assert!(store.load().unwrap().is_none());
}

#[test]
fn k04_loopback_callback_returns_code_and_checks_state() {
    let port_probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = port_probe.local_addr().unwrap().port();
    drop(port_probe);
    let redirect = format!("http://127.0.0.1:{port}/oauth/callback");
    let callback = OAuthCallbackListener::bind(&redirect).unwrap();
    let waiter =
        thread::spawn(move || callback.wait_for_code("expected-state", Duration::from_secs(5)));

    let mut stream = connect_with_retry(port);
    write!(
        stream,
        "GET /oauth/callback?code=code-123&state=expected-state HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert_eq!(waiter.join().unwrap().unwrap(), "code-123");
}

#[test]
fn k05_oauth_state_is_random_and_url_safe() {
    let first = new_oauth_state().unwrap();
    let second = new_oauth_state().unwrap();
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(first, second);
}

#[test]
fn k06_callback_ignores_wrong_state_then_accepts_real_callback() {
    let port_probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = port_probe.local_addr().unwrap().port();
    drop(port_probe);
    let redirect = format!("http://127.0.0.1:{port}/oauth/callback");
    let callback = OAuthCallbackListener::bind(&redirect).unwrap();
    let waiter =
        thread::spawn(move || callback.wait_for_code("real-state", Duration::from_secs(5)));

    send_callback(port, "code=attacker&state=wrong-state");
    send_callback(port, "code=real-code&state=real-state");
    assert_eq!(waiter.join().unwrap().unwrap(), "real-code");
}

#[test]
fn k07_expired_token_is_refreshed_and_persisted() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 8192];
        let count = stream.read(&mut request).unwrap();
        request_tx
            .send(String::from_utf8_lossy(&request[..count]).to_string())
            .unwrap();
        let body = r#"{"access_token":"access-new","expires_in":2592000,"scope":"basic netdisk"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let temp = TempDir::new();
    let store = FileBaiduTokenStore::new(temp.path().join("credentials/baidu-oauth.json"));
    store
        .save(&BaiduOAuthToken {
            access_token: "access-old".into(),
            refresh_token: Some("refresh-old".into()),
            expires_in: 1,
            scope: Some("basic netdisk".into()),
            session_key: None,
            session_secret: None,
            saved_at: "2020-01-01T00:00:00Z".into(),
            expires_at: Some("2020-01-01T00:00:01Z".into()),
        })
        .unwrap();
    let config = BaiduOAuthConfig::with_options(
        "app-key",
        "secret-key",
        "http://127.0.0.1:53682/oauth/callback",
        "basic,netdisk",
        format!("http://{address}"),
        Duration::from_secs(5),
    )
    .unwrap();
    let provider = RefreshingBaiduTokenProvider::with_refresh_before(
        store.clone(),
        BaiduOAuthClient::new(config).unwrap(),
        Duration::ZERO,
    );

    assert_eq!(provider.access_token().unwrap(), "access-new");
    let saved = store.load().unwrap().unwrap();
    assert_eq!(saved.access_token, "access-new");
    assert_eq!(saved.refresh_token.as_deref(), Some("refresh-old"));
    let request = request_rx.recv().unwrap();
    assert!(request.contains("grant_type=refresh_token"));
    assert!(request.contains("refresh_token=refresh-old"));
}

#[test]
fn k08_unexpired_token_is_used_without_refresh_request() {
    let temp = TempDir::new();
    let store = FileBaiduTokenStore::new(temp.path().join("credentials/baidu-oauth.json"));
    store
        .save(&BaiduOAuthToken {
            access_token: "access-current".into(),
            refresh_token: Some("refresh-current".into()),
            expires_in: 2_592_000,
            scope: Some("basic netdisk".into()),
            session_key: None,
            session_secret: None,
            saved_at: "2099-01-01T00:00:00Z".into(),
            expires_at: Some("2099-02-01T00:00:00Z".into()),
        })
        .unwrap();
    let config = BaiduOAuthConfig::with_options(
        "app-key",
        "secret-key",
        "http://127.0.0.1:53682/oauth/callback",
        "basic,netdisk",
        "http://127.0.0.1:9",
        Duration::from_millis(50),
    )
    .unwrap();
    let provider = RefreshingBaiduTokenProvider::new(store, BaiduOAuthClient::new(config).unwrap());

    assert_eq!(provider.access_token().unwrap(), "access-current");
}

fn connect_with_retry(port: u16) -> TcpStream {
    for _ in 0..20 {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            return stream;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("callback listener did not accept connections");
}

fn send_callback(port: u16, query: &str) {
    let mut stream = connect_with_retry(port);
    write!(
        stream,
        "GET /oauth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
}
