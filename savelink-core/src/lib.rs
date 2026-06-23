//! SaveLink 核心库（测试先行骨架）。
//!
//! 模块依赖单向向下：service → (store + repo + scan)。
//! 生产逻辑（store::FsStore、service::*）为 `todo!()`，是红灯。
//! 算法与测试支撑（scan、repo::InMemoryRepo、testkit）为已实现，是裁判工具。
//!
//! 实现顺序见 `savelink-tech-architecture.md`「落地顺序建议」；
//! 验收标准见 `savelink-restore-test-spec.md`「验收矩阵」。

pub mod error;
pub mod model;
pub mod repo;
pub mod scan;
pub mod service;
pub mod sqlite_repo;
pub mod store;

pub mod testkit;
