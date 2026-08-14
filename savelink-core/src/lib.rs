//! SaveLink 核心库（测试先行骨架）。
//!
//! 模块依赖单向向下：service → (store + repo + scan)。
//! 生产逻辑（store::FsStore、service::*）为 `todo!()`，是红灯。
//! 算法与测试支撑（scan、repo::InMemoryRepo、testkit）为已实现，是裁判工具。
//!
//! 实现顺序见 `doc/SaveLink技术架构.md`「落地顺序建议」；
//! 验收标准见 `doc/SaveLink恢复与存储测试规格.md`「验收矩阵」。

pub mod baidu_oauth;
pub mod baidu_store;
pub mod cloud_archive;
pub mod cloud_model;
pub mod cloud_protocol;
pub mod cloud_repo;
pub mod cloud_service;
pub mod cloud_store;
pub mod desmume_discovery;
pub mod error;
pub mod model;
pub mod repo;
pub mod scan;
pub mod service;
pub mod sqlite_repo;
pub mod steam_discovery;
pub mod store;
pub mod timestamp;

pub mod testkit;
