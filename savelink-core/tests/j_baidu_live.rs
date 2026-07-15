//! J 组：需要人工提供环境变量后才运行的真实百度网盘冒烟测试。

use savelink_core::baidu_store::{BaiduNetdiskConfig, BaiduNetdiskStore, StaticBaiduAccessToken};
use savelink_core::cloud_store::{CloudObjectStore, PutMode};
use savelink_core::testkit::TempDir;
use std::fs;
use std::sync::Arc;

#[test]
#[ignore = "requires SAVELINK_BAIDU_ACCESS_TOKEN and real Baidu Netdisk access"]
fn j1_real_baidu_store_uploads_lists_downloads_and_cleans_up() {
    let token = std::env::var("SAVELINK_BAIDU_ACCESS_TOKEN")
        .expect("SAVELINK_BAIDU_ACCESS_TOKEN is required for this ignored test");
    let config = BaiduNetdiskConfig {
        logical_root: "savelink-adapter-smoke".into(),
        remote_root: "/apps/savelink/savelink-adapter-smoke".into(),
        ..BaiduNetdiskConfig::default()
    };
    let store = BaiduNetdiskStore::with_config(
        Arc::new(StaticBaiduAccessToken::new(token).unwrap()),
        config,
    )
    .unwrap();
    let remote_path = format!("savelink-adapter-smoke/process-{}.bin", std::process::id());
    let mut cleanup = RemoteCleanup::new(&store, &remote_path);
    let tmp = TempDir::new();
    let source = tmp.path().join("source.bin");
    let downloaded = tmp.path().join("downloaded.bin");
    let payload = b"SaveLink BaiduNetdiskStore live smoke";
    fs::write(&source, payload).unwrap();

    let uploaded = store
        .put_file(&remote_path, &source, PutMode::Overwrite)
        .unwrap();
    assert_eq!(uploaded.size, payload.len() as u64);
    assert_eq!(
        store.stat_file(&remote_path).unwrap().unwrap().size,
        payload.len() as u64
    );
    assert!(store
        .list_directory("savelink-adapter-smoke")
        .unwrap()
        .iter()
        .any(|entry| entry.path == remote_path));

    store.get_file(&remote_path, &downloaded).unwrap();
    assert_eq!(fs::read(&downloaded).unwrap(), payload);
    store.delete_file(&remote_path).unwrap();
    cleanup.disarm();
    assert_eq!(store.stat_file(&remote_path).unwrap(), None);
}

struct RemoteCleanup<'a> {
    store: &'a BaiduNetdiskStore,
    remote_path: &'a str,
    armed: bool,
}

impl<'a> RemoteCleanup<'a> {
    fn new(store: &'a BaiduNetdiskStore, remote_path: &'a str) -> Self {
        Self {
            store,
            remote_path,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RemoteCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.store.delete_file(self.remote_path);
        }
    }
}
