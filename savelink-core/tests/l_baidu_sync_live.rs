//! L 组：真实百度网盘只读发现与设备 B 接收落地。
//!
//! 默认忽略。要求云端已有 SaveLink v1 快照，并通过环境变量提供 access token。

use savelink_core::baidu_store::{BaiduNetdiskStore, StaticBaiduAccessToken};
use savelink_core::cloud_archive::ZipCloudArchiveCodec;
use savelink_core::cloud_service::{CloudSyncService, ReceiveOutcome};
use savelink_core::cloud_store::CloudObjectStore;
use savelink_core::repo::{Clock, Repository};
use savelink_core::sqlite_repo::SqliteRepo;
use savelink_core::store::{FsStore, SnapshotStore};
use savelink_core::testkit::TempDir;
use std::env;
use std::sync::Arc;

struct LiveClock;

impl Clock for LiveClock {
    fn now_stamp(&self) -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

#[test]
#[ignore = "requires SAVELINK_BAIDU_ACCESS_TOKEN and an existing SaveLink v1 cloud snapshot"]
fn l1_real_baidu_service_discovers_and_receives_existing_snapshot() {
    let token = env::var("SAVELINK_BAIDU_ACCESS_TOKEN")
        .expect("set SAVELINK_BAIDU_ACCESS_TOKEN before running this ignored test");
    let cloud: Arc<dyn CloudObjectStore> = Arc::new(
        BaiduNetdiskStore::new(Arc::new(StaticBaiduAccessToken::new(token).unwrap())).unwrap(),
    );
    let temp = TempDir::new();
    let repo = Arc::new(SqliteRepo::open(temp.path().join("device-b.db")).unwrap());
    let store = Arc::new(FsStore::new(temp.path().join("repository")));
    let service = CloudSyncService::new(
        repo.clone(),
        store.clone(),
        cloud,
        Arc::new(ZipCloudArchiveCodec::new()),
        Arc::new(LiveClock),
        temp.path().join("cloud-work"),
        "baidu-live-device-b",
        "device_b_live_test",
        "repo_b_live_test",
    )
    .unwrap();

    let discovered = service.discover_remote_catalog().unwrap();
    assert!(!discovered.is_empty(), "真实百度目录中应至少有一条快照");
    assert!(
        repo.list_games().unwrap().is_empty(),
        "只读发现不能创建本机游戏"
    );

    let target = &discovered[0];
    assert_eq!(
        service
            .receive_remote_snapshot(&target.snapshot.snapshot_id)
            .unwrap(),
        ReceiveOutcome::Downloaded
    );
    let local = repo
        .get_snapshot(&target.snapshot.snapshot_id)
        .unwrap()
        .expect("下载后应存在本机快照记录");
    assert!(store.verify(&local.storage_key).unwrap());
    let game = repo
        .get_game(&target.cloud_game_id)
        .unwrap()
        .expect("下载后应创建未绑定的本机游戏");
    assert!(game.save_paths.is_empty(), "云端不得带入设备 A 的真实路径");
}
