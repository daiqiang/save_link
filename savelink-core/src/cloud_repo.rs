//! 云同步本机状态持久化抽象。

use crate::cloud_model::{CloudAccount, CloudGameBinding, CloudSnapshotRecord, CloudSyncStatus};
use crate::error::Result;

pub trait CloudStateRepository: Send + Sync {
    fn set_setting(&self, key: &str, value: &str) -> Result<()>;
    fn get_setting(&self, key: &str) -> Result<Option<String>>;

    fn upsert_cloud_account(&self, account: CloudAccount) -> Result<()>;
    fn get_cloud_account(&self, account_id: &str) -> Result<Option<CloudAccount>>;
    fn list_cloud_accounts(&self) -> Result<Vec<CloudAccount>>;

    fn upsert_cloud_game_binding(&self, binding: CloudGameBinding) -> Result<()>;
    fn get_cloud_game_binding(
        &self,
        account_id: &str,
        cloud_game_id: &str,
    ) -> Result<Option<CloudGameBinding>>;
    fn list_cloud_game_bindings(&self, account_id: &str) -> Result<Vec<CloudGameBinding>>;

    fn upsert_cloud_snapshot(&self, snapshot: CloudSnapshotRecord) -> Result<()>;
    fn get_cloud_snapshot(
        &self,
        account_id: &str,
        snapshot_id: &str,
    ) -> Result<Option<CloudSnapshotRecord>>;
    fn list_cloud_snapshots(
        &self,
        account_id: &str,
        cloud_game_id: &str,
    ) -> Result<Vec<CloudSnapshotRecord>>;
    fn list_cloud_snapshots_by_status(
        &self,
        account_id: &str,
        status: CloudSyncStatus,
    ) -> Result<Vec<CloudSnapshotRecord>>;
    fn update_cloud_snapshot_status(
        &self,
        account_id: &str,
        snapshot_id: &str,
        status: CloudSyncStatus,
        last_synced_at: Option<&str>,
        last_error_code: Option<&str>,
    ) -> Result<()>;
}
