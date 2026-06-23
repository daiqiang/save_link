//! 目录扫描 + content_hash 计算。
//!
//! 本模块是**已实现且权威**的——不是 todo。原因：测试夹具 `dir_fingerprint`
//! 必须与生产 `content_hash` 用同一套算法，否则 A2「未变化判断」与
//! B3「内容一致」的断言会假阳/假阴（见测试规格 D3）。
//! 因此把唯一的算法实现放在这里，测试与生产共用 `fingerprint_dir`。
//!
//! 哈希算法：零依赖的 FNV-1a（64-bit）。它不是密码学安全的，
//! 但对「检测存档是否变化」「比对恢复内容是否一致」足够，且可离线、可重复。
//! 实现者若引入更强哈希（如 blake3），**必须同时改这一处**，测试自动跟随。

use crate::error::{Result, SaveLinkError};
use crate::model::ScanResult;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// 收集目录下所有文件的「相对路径 + 内容」，按相对路径排序后逐项累计哈希。
///
/// 排序保证哈希与文件枚举顺序无关（测试规格 D3）。
/// 相对路径用 `/` 归一，保证跨平台（Windows 反斜杠）结果一致（D5）。
/// 任一文件内容、文件名、新增/删除都会改变最终指纹（A3）。
pub fn fingerprint_dir(dir: &Path) -> Result<ScanResult> {
    if !dir.exists() {
        return Err(SaveLinkError::SaveDirMissing);
    }
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(dir, &mut files).map_err(|e| SaveLinkError::Io(e.to_string()))?;

    // 用归一化后的相对路径排序，保证顺序无关。
    let mut entries: Vec<(String, PathBuf)> = files
        .iter()
        .map(|p| (rel_key(dir, p), p.clone()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut h = FNV_OFFSET;
    let mut total_size: u64 = 0;
    let file_count = entries.len() as u64;

    for (key, path) in &entries {
        // 路径计入哈希：保证改名/移动也算变化。
        h = fnv1a(h, key.as_bytes());
        h = fnv1a(h, &[0u8]); // 分隔符，避免拼接歧义

        let mut f = fs::File::open(path).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf).map_err(|e| SaveLinkError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            total_size += n as u64;
            h = fnv1a(h, &buf[..n]);
        }
        h = fnv1a(h, &[0xffu8]); // 文件结束标记
    }

    Ok(ScanResult {
        file_count,
        total_size,
        content_hash: format!("{h:016x}"),
        readable: true,
    })
}

/// 相对路径归一化为以 `/` 分隔的字符串。
fn rel_key(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// 生产扫描入口。MVP 直接复用 `fingerprint_dir`。
/// 实现者后续接入 include/exclude glob 时在此扩展（model 已预留字段）。
pub fn scan(save_paths: &[PathBuf]) -> Result<ScanResult> {
    // 多目录：MVP 先支持单目录，结构上预留聚合。
    // 这里给出可用实现，便于 create_snapshot 的"测试读取"与 NoChange 判断。
    if save_paths.is_empty() {
        return Ok(ScanResult {
            file_count: 0,
            total_size: 0,
            content_hash: format!("{FNV_OFFSET:016x}"),
            readable: true,
        });
    }
    // 单目录路径：直接指纹。多目录聚合留给实现者（不影响当前测试范围）。
    fingerprint_dir(&save_paths[0])
}
