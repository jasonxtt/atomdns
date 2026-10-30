//! The WebUI directory `<root>/ui/main` in an update: an online update replaces it with
//! the release's `atomdns-webui.tar.gz` **only when it already exists** as a real directory,
//! inside the same transaction as the program and the configuration package.
//!
//! Installing is two renames — the old directory into the transaction (`webui/previous`),
//! then the freshly unpacked one (`webui/new`) into place — and [`restore`] undoes them
//! from whatever point it finds: if `webui/previous` exists the old directory goes back,
//! otherwise it was never moved. Both are repeatable after an interruption.

use std::path::{Path, PathBuf};

use super::archive::{TreeLimits, extract_tar_gz_tree};
use super::fsio::sync_dir;
use super::transaction::Transaction;

/// The unpacked size limits of a WebUI archive (the static server serves at most
/// 16 MiB per file).
pub(crate) const LIMITS: TreeLimits = TreeLimits {
    max_entries: 4096,
    max_depth: 8,
    max_file: 16 << 20,
    max_total: 64 << 20,
};

/// `<root>/ui/main`.
pub(crate) fn target(root: &Path) -> PathBuf {
    root.join("ui").join(crate::external_ui::MAIN)
}

/// What `<root>/ui/main` is when an update starts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// A real directory: it is replaced.
    Directory,
    /// Not there (or no `ui/` at all): nothing is downloaded or created.
    Absent,
    /// A symbolic link or not a directory: left alone (the host does not serve it).
    Unusable,
}

pub(crate) fn presence(root: &Path) -> Presence {
    let ui = root.join("ui");
    match std::fs::symlink_metadata(&ui) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Presence::Unusable,
        Err(_) => return Presence::Absent,
    }
    match std::fs::symlink_metadata(target(root)) {
        Ok(metadata) if metadata.is_dir() => Presence::Directory,
        Ok(_) => Presence::Unusable,
        Err(_) => Presence::Absent,
    }
}

/// Unpacks `archive` into `scratch` and checks that it is a WebUI package: a regular
/// `index.html` and a `ui-manifest.json` with `manifest_schema` 1 and a `ui_version`.
/// The interface level is not judged here (the old program's level says nothing about
/// the new one's); the candidate's own registry judges it, and the guard waits for it.
/// `scratch` is removed in every case.
pub(crate) fn check_archive(archive: &[u8], scratch: &Path) -> Result<(), String> {
    let result = extract_tar_gz_tree(archive, scratch, LIMITS)
        .map_err(|reason| format!("界面资产无法解压: {reason}"))
        .and_then(|()| check_tree(scratch));
    let _ = std::fs::remove_dir_all(scratch);
    result
}

fn check_tree(dir: &Path) -> Result<(), String> {
    let regular = |name: &str| {
        std::fs::symlink_metadata(dir.join(name)).is_ok_and(|metadata| metadata.is_file())
    };
    if !regular("index.html") {
        return Err("界面资产缺少 index.html".to_owned());
    }
    if !regular("ui-manifest.json") {
        return Err("界面资产缺少 ui-manifest.json".to_owned());
    }
    let manifest: serde_json::Value = std::fs::read(dir.join("ui-manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("界面资产的 ui-manifest.json 不是合法的 JSON")?;
    if manifest["manifest_schema"] != 1
        || manifest["ui_version"]
            .as_str()
            .is_none_or(|version| version.trim().is_empty())
    {
        return Err("界面资产的 ui-manifest.json 缺少 manifest_schema 1 或 ui_version".to_owned());
    }
    Ok(())
}

/// Whether `a` and `b` are on the same filesystem (a rename between them is atomic).
pub(crate) fn same_filesystem(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(a)?.dev() == std::fs::metadata(b)?.dev())
}

fn sync_parents(paths: &[&Path]) -> std::io::Result<()> {
    for path in paths {
        if let Some(parent) = path.parent() {
            sync_dir(parent)?;
        }
    }
    Ok(())
}

/// What `path` is, without following a symbolic link: `None` when it does not exist; any
/// other failure to look is an error (never taken for "absent").
fn inspect(path: &Path) -> Result<Option<std::fs::Metadata>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法检查 {}: {error}", path.display())),
    }
}

/// `<root>/ui` must still be a real directory (not a symbolic link) whenever `ui/main` is
/// moved, so a rename never reaches a directory outside the configuration root.
fn real_ui_directory(root: &Path) -> Result<(), String> {
    match inspect(&root.join("ui"))? {
        Some(metadata) if metadata.is_dir() => Ok(()),
        _ => Err("配置目录下的 ui/ 已不是普通目录（可能被换成符号链接），拒绝移动界面".to_owned()),
    }
}

/// Puts the release's WebUI in place (guard, `installing`): re-checks the staged
/// archive's sha256, unpacks it into `webui/new`, moves the old `ui/main` to
/// `webui/previous` and the new tree to `ui/main`.
pub(crate) fn install(tx: &Transaction) -> Result<(), String> {
    if tx.webui_archive_path.is_empty() {
        return Ok(());
    }
    let root = Path::new(&tx.config_base_dir);
    let live = target(root);
    let new = Path::new(&tx.webui_new_dir);
    let previous = Path::new(&tx.webui_previous_dir);
    let bytes = std::fs::read(&tx.webui_archive_path)
        .map_err(|error| format!("读取界面资产失败: {error}"))?;
    if super::sha256_hex(&bytes) != tx.webui_sha256 {
        return Err("界面资产的校验值与暂存时不同".to_owned());
    }
    match std::fs::remove_dir_all(new) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("清理界面暂存目录失败: {error}")),
    }
    let staging = new.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(staging).map_err(|error| format!("创建界面暂存目录失败: {error}"))?;
    extract_tar_gz_tree(&bytes, new, LIMITS)
        .map_err(|reason| format!("解压界面资产失败: {reason}"))?;
    check_tree(new)?;
    real_ui_directory(root)?;
    if !same_filesystem(&root.join("ui"), staging).unwrap_or(false) {
        return Err("ui/ 与 update/ 不在同一文件系统，无法原子替换界面".to_owned());
    }
    if inspect(previous)?.is_none() {
        match inspect(&live)? {
            Some(metadata) if metadata.is_dir() => {}
            _ => return Err("ui/main 已不是目录，拒绝替换".to_owned()),
        }
        std::fs::rename(&live, previous).map_err(|error| format!("移开旧界面失败: {error}"))?;
        sync_parents(&[&live, previous]).map_err(|error| format!("同步目录失败: {error}"))?;
    }
    std::fs::rename(new, &live).map_err(|error| format!("放入新界面失败: {error}"))?;
    sync_parents(&[&live, new]).map_err(|error| format!("同步目录失败: {error}"))
}

/// Puts the old `ui/main` back (rollback). Idempotent: without `webui/previous` the old
/// directory was never moved and nothing changes. A failure to look at either directory
/// is an error, so the rollback is not recorded as done and is tried again at the next
/// start.
pub(crate) fn restore(tx: &Transaction) -> Result<(), String> {
    if tx.webui_archive_path.is_empty() {
        return Ok(());
    }
    let root = Path::new(&tx.config_base_dir);
    let live = target(root);
    let previous = Path::new(&tx.webui_previous_dir);
    if inspect(previous)?.is_none() {
        return Ok(());
    }
    real_ui_directory(root)?;
    if inspect(&live)?.is_some() {
        let parent = previous.parent().unwrap_or_else(|| Path::new("."));
        let mut failed = None;
        for index in 0..1000 {
            let path = parent.join(format!("failed-{index}"));
            if inspect(&path)?.is_none() {
                failed = Some(path);
                break;
            }
        }
        let failed = failed.ok_or("没有可用的位置移开新界面")?;
        std::fs::rename(&live, &failed).map_err(|error| format!("移开新界面失败: {error}"))?;
    }
    std::fs::rename(previous, &live).map_err(|error| format!("恢复旧界面失败: {error}"))?;
    sync_parents(&[&live, previous]).map_err(|error| format!("同步目录失败: {error}"))
}
