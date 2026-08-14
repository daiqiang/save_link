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
use crate::model::{SaveFileMapping, SaveSource, ScanResult};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

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
    let mut entries: Vec<(String, PathBuf)> =
        files.iter().map(|p| (rel_key(dir, p), p.clone())).collect();
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
            let n = f
                .read(&mut buf)
                .map_err(|e| SaveLinkError::Io(e.to_string()))?;
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

/// 扫描共享目录中属于某个游戏的精确文件，并以快照逻辑路径参与指纹计算。
pub fn fingerprint_selected_files(root: &Path, mappings: &[SaveFileMapping]) -> Result<ScanResult> {
    if !root.is_dir() {
        return Err(SaveLinkError::SaveDirMissing);
    }
    validate_file_mappings(mappings)?;
    let mut entries = Vec::new();
    for mapping in mappings {
        let local = root.join(&mapping.local_relative_path);
        if !local.exists() {
            continue;
        }
        if !local.is_file() {
            return Err(SaveLinkError::SaveDirUnreadable);
        }
        entries.push((path_key(&mapping.snapshot_relative_path), local));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    fingerprint_entries(&entries)
}

fn fingerprint_entries(entries: &[(String, PathBuf)]) -> Result<ScanResult> {
    let mut hash = FNV_OFFSET;
    let mut total_size = 0u64;
    for (key, path) in entries {
        hash = fnv1a(hash, key.as_bytes());
        hash = fnv1a(hash, &[0]);
        let mut file = fs::File::open(path).map_err(|e| SaveLinkError::Io(e.to_string()))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|e| SaveLinkError::Io(e.to_string()))?;
            if count == 0 {
                break;
            }
            total_size += count as u64;
            hash = fnv1a(hash, &buffer[..count]);
        }
        hash = fnv1a(hash, &[0xff]);
    }
    Ok(ScanResult {
        file_count: entries.len() as u64,
        total_size,
        content_hash: format!("{hash:016x}"),
        readable: true,
    })
}

/// 相对路径归一化为以 `/` 分隔的字符串。
fn rel_key(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    path_key(rel)
}

fn path_key(path: &Path) -> String {
    path.components()
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
    if save_paths.is_empty() {
        return Ok(ScanResult {
            file_count: 0,
            total_size: 0,
            content_hash: format!("{FNV_OFFSET:016x}"),
            readable: true,
        });
    }
    if save_paths.len() == 1 {
        return fingerprint_dir(&save_paths[0]);
    }
    fingerprint_sources(save_paths)
}

/// 扫描游戏的有效来源。传统整目录来源保持原指纹算法；精确文件来源使用快照
/// 逻辑路径，因此同一 ROM 改名后仍能与原快照比较。
pub fn scan_save_sources(sources: &[SaveSource]) -> Result<ScanResult> {
    if sources.is_empty() {
        return scan(&[]);
    }
    validate_save_sources(sources)?;
    if sources.iter().all(SaveSource::is_directory) {
        let paths = sources
            .iter()
            .map(|source| source.root().to_path_buf())
            .collect::<Vec<_>>();
        return scan(&paths);
    }

    let fingerprints = sources
        .iter()
        .map(|source| match source {
            SaveSource::Directory { path } => fingerprint_dir(path),
            SaveSource::Files { root, files } => fingerprint_selected_files(root, files),
        })
        .collect::<Result<Vec<_>>>()?;
    aggregate_source_fingerprints(&fingerprints)
}

pub fn validate_save_sources(sources: &[SaveSource]) -> Result<()> {
    let roots = sources
        .iter()
        .map(|source| source.root().to_path_buf())
        .collect::<Vec<_>>();
    validate_save_paths(&roots)?;
    for source in sources {
        if let SaveSource::Files { files, .. } = source {
            validate_file_mappings(files)?;
        }
    }
    Ok(())
}

fn validate_file_mappings(mappings: &[SaveFileMapping]) -> Result<()> {
    if mappings.is_empty() {
        return Err(SaveLinkError::Io("精确文件来源不能为空".into()));
    }
    let mut local_paths = BTreeSet::new();
    let mut snapshot_paths = BTreeSet::new();
    for mapping in mappings {
        if !is_safe_relative_path(&mapping.local_relative_path)
            || !is_safe_relative_path(&mapping.snapshot_relative_path)
        {
            return Err(SaveLinkError::Io("存档文件映射包含非法相对路径".into()));
        }
        let local = path_key(&mapping.local_relative_path).to_ascii_lowercase();
        let snapshot = path_key(&mapping.snapshot_relative_path).to_ascii_lowercase();
        if !local_paths.insert(local) || !snapshot_paths.insert(snapshot) {
            return Err(SaveLinkError::Io("存档文件映射包含重复路径".into()));
        }
    }
    Ok(())
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn aggregate_source_fingerprints(fingerprints: &[ScanResult]) -> Result<ScanResult> {
    if fingerprints.is_empty() {
        return scan(&[]);
    }
    if fingerprints.len() == 1 {
        return Ok(fingerprints[0].clone());
    }
    let mut hash = FNV_OFFSET;
    let mut file_count = 0u64;
    let mut total_size = 0u64;
    for (index, child) in fingerprints.iter().enumerate() {
        hash = fnv1a(hash, b"source\0");
        hash = fnv1a(hash, index.to_string().as_bytes());
        hash = fnv1a(hash, &[0]);
        hash = fnv1a(hash, child.content_hash.as_bytes());
        hash = fnv1a(hash, &[0xff]);
        file_count += child.file_count;
        total_size += child.total_size;
    }
    Ok(ScanResult {
        file_count,
        total_size,
        content_hash: format!("{hash:016x}"),
        readable: true,
    })
}

/// 检查多个存档根目录是否可以作为相互独立的来源处理。
///
/// 多目录快照、恢复和云端载荷都依赖每个来源拥有独立的根目录。父子目录
/// 会导致同一批文件被重复扫描，并破坏恢复时的目录替换顺序，因此在进入
/// 这些流程前统一拒绝。
pub fn validate_save_paths(save_paths: &[PathBuf]) -> Result<()> {
    for (index, first) in save_paths.iter().enumerate() {
        for second in save_paths.iter().skip(index + 1) {
            if is_same_or_ancestor(first, second) || is_same_or_ancestor(second, first) {
                return Err(SaveLinkError::OverlappingSavePaths {
                    first: first.clone(),
                    second: second.clone(),
                });
            }
        }
    }
    Ok(())
}

fn is_same_or_ancestor(parent: &Path, candidate: &Path) -> bool {
    let parent = normalized_path_components(parent);
    let candidate = normalized_path_components(candidate);
    candidate.len() >= parent.len() && candidate.starts_with(&parent)
}

fn normalized_path_components(path: &Path) -> Vec<String> {
    let mut components = Vec::new();
    for component in path.to_string_lossy().replace('\\', "/").split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value.to_ascii_lowercase()),
        }
    }
    components
}

/// 对多个独立存档根目录生成一个稳定的聚合指纹。
///
/// 每个根目录先沿用单目录指纹算法，再把目录序号和子指纹按顺序聚合；因此同名文件
/// 位于不同存档根时不会冲突，任意一个根目录的增删改都会改变整快照指纹。
pub fn fingerprint_sources(sources: &[PathBuf]) -> Result<ScanResult> {
    if sources.len() <= 1 {
        return scan(sources);
    }
    validate_save_paths(sources)?;
    let mut hash = FNV_OFFSET;
    let mut file_count = 0u64;
    let mut total_size = 0u64;
    for (index, source) in sources.iter().enumerate() {
        let child = fingerprint_dir(source)?;
        hash = fnv1a(hash, b"source\0");
        hash = fnv1a(hash, index.to_string().as_bytes());
        hash = fnv1a(hash, &[0]);
        hash = fnv1a(hash, child.content_hash.as_bytes());
        hash = fnv1a(hash, &[0xff]);
        file_count += child.file_count;
        total_size += child.total_size;
    }
    Ok(ScanResult {
        file_count,
        total_size,
        content_hash: format!("{hash:016x}"),
        readable: true,
    })
}

/// 校验云端/仓库中的标准快照载荷。多目录布局固定为 `sources/{index}/...`。
pub fn fingerprint_snapshot_payload(root: &Path, source_count: u32) -> Result<ScanResult> {
    if source_count <= 1 {
        return fingerprint_dir(root);
    }
    let sources = (0..source_count)
        .map(|index| root.join("sources").join(index.to_string()))
        .collect::<Vec<_>>();
    fingerprint_sources(&sources)
}
