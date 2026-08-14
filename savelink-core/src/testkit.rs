//! 测试支撑工具（**已实现**）。
//!
//! 这是「让规格拿着就能用」的关键：故障注入接缝与裁判夹具都在这里写死，
//! 实现者/agent 无法把危险用例偷换成 happy-path。
//!
//! 提供：
//! - `make_save_dir` / `write_files`：构造存档目录。
//! - `dir_fingerprint`：内容裁判（复用生产 `scan` 的同一算法，杜绝假阳/假阴）。
//! - `FailingStore`：包装任意 `SnapshotStore`，在指定方法上按需注入失败/panic，
//!   用于 A4 / B2 / B4 / B5 / C3 等故障路径。
//! - `corrupt_dir`：破坏某快照物理内容，模拟损坏（B4 / D2）。

use crate::error::{Result, SaveLinkError};
use crate::model::{SaveSource, ScanResult, StoredSnapshot};
use crate::scan;
use crate::store::SnapshotStore;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 在 `root` 下按 (相对路径, 内容) 写出文件，自动建子目录。
pub fn write_files(root: &Path, files: &[(&str, &[u8])]) {
    for (rel, content) in files {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
    }
}

/// 内容裁判：与生产 `content_hash` 完全同算法。
/// B3 等「恢复后内容一致」断言用它比对，保证不会因算法不一致而误判。
pub fn dir_fingerprint(dir: &Path) -> String {
    scan::fingerprint_dir(dir)
        .map(|r| r.content_hash)
        .unwrap_or_else(|_| "<unreadable>".to_string())
}

/// 故障注入点：包装 store 的哪个方法、第几次调用时失败、以何种方式失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOp {
    Create,
    Restore,
    Verify,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// 返回 Io 错误（可恢复失败路径）。
    Error,
    /// 在写到一半时返回错误（用于"半成品/中途崩溃"语义；
    /// 配合 FsStore 的分步实现，模拟覆盖阶段中断 → B5）。
    PartialThenError,
    /// 直接 panic（模拟进程崩溃 → B5 / E1 配合子进程或 catch_unwind）。
    Panic,
}

/// 包装任意 SnapshotStore，对指定操作注入故障。
///
/// 用法：`FailingStore::new(inner).fail(FailOp::Create, FailKind::Error)`
/// 默认不注入任何故障，行为完全透传到 inner。
pub struct FailingStore {
    inner: Arc<dyn SnapshotStore>,
    op: Option<FailOp>,
    kind: FailKind,
    /// 第几次命中该 op 时触发（从 1 计）。0 表示每次都触发。
    trigger_on_call: usize,
    counter: AtomicUsize,
}

impl FailingStore {
    pub fn new(inner: Arc<dyn SnapshotStore>) -> Self {
        Self {
            inner,
            op: None,
            kind: FailKind::Error,
            trigger_on_call: 0,
            counter: AtomicUsize::new(0),
        }
    }

    /// 设定在某操作上注入某种故障（每次命中都触发）。
    pub fn fail(mut self, op: FailOp, kind: FailKind) -> Self {
        self.op = Some(op);
        self.kind = kind;
        self.trigger_on_call = 0;
        self
    }

    /// 仅在第 n 次命中该 op 时触发（便于 B8：第一次恢复成功、第二次才注入等场景）。
    pub fn fail_on_call(mut self, op: FailOp, kind: FailKind, nth: usize) -> Self {
        self.op = Some(op);
        self.kind = kind;
        self.trigger_on_call = nth;
        self
    }

    fn should_fail(&self, op: FailOp) -> bool {
        if self.op != Some(op) {
            return false;
        }
        if self.trigger_on_call == 0 {
            return true;
        }
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        n == self.trigger_on_call
    }

    fn trip(&self) -> Result<()> {
        match self.kind {
            FailKind::Error | FailKind::PartialThenError => {
                Err(SaveLinkError::Io("injected failure".into()))
            }
            FailKind::Panic => panic!("injected panic (simulated crash)"),
        }
    }
}

impl SnapshotStore for FailingStore {
    fn create(
        &self,
        snapshot_id: &str,
        sources: &[PathBuf],
        ctx: &ScanResult,
    ) -> Result<StoredSnapshot> {
        if self.should_fail(FailOp::Create) {
            // PartialThenError 语义下，实现者的 FsStore 应在写一半后才暴露错误；
            // 这里先 trip，由被测的事务逻辑保证不留半成品（A4）。
            self.trip()?;
        }
        self.inner.create(snapshot_id, sources, ctx)
    }

    fn create_save_sources(
        &self,
        snapshot_id: &str,
        sources: &[SaveSource],
        ctx: &ScanResult,
    ) -> Result<StoredSnapshot> {
        if self.should_fail(FailOp::Create) {
            self.trip()?;
        }
        self.inner.create_save_sources(snapshot_id, sources, ctx)
    }

    fn restore(&self, key: &str, target: &Path) -> Result<()> {
        if self.should_fail(FailOp::Restore) {
            self.trip()?;
        }
        self.inner.restore(key, target)
    }

    fn restore_sources(&self, key: &str, targets: &[PathBuf]) -> Result<()> {
        if self.should_fail(FailOp::Restore) {
            self.trip()?;
        }
        self.inner.restore_sources(key, targets)
    }

    fn verify(&self, key: &str) -> Result<bool> {
        if self.should_fail(FailOp::Verify) {
            self.trip()?;
        }
        self.inner.verify(key)
    }

    fn delete(&self, key: &str) -> Result<()> {
        if self.should_fail(FailOp::Delete) {
            self.trip()?;
        }
        self.inner.delete(key)
    }
}

/// 破坏目录下所有文件内容（模拟快照损坏）。配合 FsStore 的存储布局使用。
pub fn corrupt_dir(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                corrupt_dir(&p);
            } else {
                let _ = std::fs::write(&p, b"CORRUPTED");
            }
        }
    }
}

/// 零依赖的临时目录：在系统 temp 下建唯一目录，Drop 时递归删除。
/// 用于替代 `tempfile` crate，保持骨架可离线编译。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("savelink_test_{pid}_{nanos}_{n}"));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 在该临时目录下建一个子目录并返回其路径。
    pub fn child(&self, name: &str) -> PathBuf {
        let p = self.path.join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Default for TempDir {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
