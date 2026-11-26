//! The WebUI directories `<root>/ui/main` (default at `/`) and `<root>/ui/lg` (optional
//! glass UI at `/lg/`) in an update: an online update replaces `ui/main` with the single
//! `atomdns-webui.tar.gz` **only when `ui/main` already exists** as a real directory. When
//! that archive also carries `bundled-ui/lg/`, the glass UI is split out and installed or
//! replaced at `ui/lg` inside the same transaction.
//!
//! When an older updater (e.g. v0.7.0) unpacked `atomdns-webui.tar.gz` directly into
//! `ui/main`, [`promote_bundled_on_start`] runs on normal startup (never during an update
//! guard trial) before the UI registry scans `ui/`, atomically moving
//! `ui/main/bundled-ui/lg` into `ui/lg` so a single upgrade delivers `/lg/` immediately.
//!
//! Installing an existing directory is two renames — the old directory into the transaction
//! (`previous`), then the freshly unpacked one (`new`) into place; installing a fresh `ui/lg`
//! is one rename (`new` → `ui/lg`). [`restore`] undoes both from whatever point it finds:
//! if `previous` exists the old directory goes back, and a newly created `ui/lg` is removed.
//! Both are repeatable after an interruption.

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

/// Top-level directory inside `atomdns-webui.tar.gz` holding bundled alternate UIs
/// (`bundled-ui/lg/` for `<root>/ui/lg/`).
pub(crate) const BUNDLED_UI_DIR: &str = "bundled-ui";

/// `<root>/ui/<name>`.
pub(crate) fn target_named(root: &Path, name: &str) -> PathBuf {
    root.join("ui").join(name)
}

/// `<root>/ui/main`.
pub(crate) fn target(root: &Path) -> PathBuf {
    target_named(root, crate::external_ui::MAIN)
}

/// `<root>/ui/lg`.
pub(crate) fn target_glass(root: &Path) -> PathBuf {
    target_named(root, crate::external_ui::GLASS)
}

/// `<main_dir>/bundled-ui/lg`.
pub(crate) fn bundled_glass_path(main_dir: &Path) -> PathBuf {
    main_dir
        .join(BUNDLED_UI_DIR)
        .join(crate::external_ui::GLASS)
}

/// What `<root>/ui/<name>` is when an update starts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// A real directory: it is replaced.
    Directory,
    /// Not there (or no `ui/` at all).
    Absent,
    /// A symbolic link or not a directory: left alone (the host does not serve it).
    Unusable,
}

pub(crate) fn presence_named(root: &Path, name: &str) -> Presence {
    let ui = root.join("ui");
    match std::fs::symlink_metadata(&ui) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Presence::Unusable,
        Err(_) => return Presence::Absent,
    }
    match std::fs::symlink_metadata(target_named(root, name)) {
        Ok(metadata) if metadata.is_dir() => Presence::Directory,
        Ok(_) => Presence::Unusable,
        Err(_) => Presence::Absent,
    }
}

pub(crate) fn presence(root: &Path) -> Presence {
    presence_named(root, crate::external_ui::MAIN)
}

pub(crate) fn presence_glass(root: &Path) -> Presence {
    presence_named(root, crate::external_ui::GLASS)
}

/// Unpacks `archive` into `scratch` and checks that it is a WebUI package: a regular
/// `index.html` and a `ui-manifest.json` with `manifest_schema` 1 and a `ui_version`.
/// If `bundled-ui/lg/` is present, validates it by the same rules and returns `Ok(true)`.
/// The interface level is not judged here (the old program's level says nothing about
/// the new one's); the candidate's own registry judges it, and the guard waits for it.
/// `scratch` is removed in every case.
pub(crate) fn check_archive(archive: &[u8], scratch: &Path) -> Result<bool, String> {
    let result = extract_tar_gz_tree(archive, scratch, LIMITS)
        .map_err(|reason| format!("界面资产无法解压: {reason}"))
        .and_then(|()| {
            check_tree(scratch)?;
            let bundled_lg = bundled_glass_path(scratch);
            match inspect(&bundled_lg)? {
                Some(metadata) if metadata.is_dir() => {
                    check_tree(&bundled_lg)?;
                    Ok(true)
                }
                Some(_) => Err("界面资产里的 bundled-ui/lg 不是普通目录".to_owned()),
                None => Ok(false),
            }
        });
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

/// `<root>/ui` must still be a real directory (not a symbolic link) whenever `ui/main` or
/// `ui/lg` is moved, so a rename never reaches a directory outside the configuration root.
fn real_ui_directory(root: &Path) -> Result<(), String> {
    match inspect(&root.join("ui"))? {
        Some(metadata) if metadata.is_dir() => Ok(()),
        _ => Err("配置目录下的 ui/ 已不是普通目录（可能被换成符号链接），拒绝移动界面".to_owned()),
    }
}

fn remove_dir_if_exists(path: &Path, label: &str) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理{label}失败: {error}")),
    }
}

/// Splits `<unpacked_main>/bundled-ui/lg` out into `glass_dest` (when `Some`) and strips
/// `<unpacked_main>/bundled-ui` so `ui/main` never contains bundled alternate UI files.
/// Returns `Ok(true)` when a valid `bundled-ui/lg` was moved to `glass_dest`.
pub(crate) fn split_bundled_glass(
    unpacked_main: &Path,
    glass_dest: Option<&Path>,
) -> Result<bool, String> {
    let bundled_root = unpacked_main.join(BUNDLED_UI_DIR);
    let bundled_lg = bundled_glass_path(unpacked_main);
    let extracted = match inspect(&bundled_lg)? {
        Some(metadata) if metadata.is_dir() => {
            if let Some(dest) = glass_dest {
                check_tree(&bundled_lg)?;
                remove_dir_if_exists(dest, "玻璃界面暂存目录")?;
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("创建玻璃界面暂存目录失败: {error}"))?;
                }
                std::fs::rename(&bundled_lg, dest)
                    .map_err(|error| format!("拆分玻璃界面失败: {error}"))?;
                sync_parents(&[&bundled_lg, dest])
                    .map_err(|error| format!("同步目录失败: {error}"))?;
                true
            } else {
                false
            }
        }
        Some(_) => return Err("界面资产里的 bundled-ui/lg 不是普通目录".to_owned()),
        None => false,
    };
    if inspect(&bundled_root)?.is_some() {
        remove_dir_if_exists(&bundled_root, "内嵌界面暂存目录")?;
        sync_dir(unpacked_main).map_err(|error| format!("同步目录失败: {error}"))?;
    }
    Ok(extracted)
}

fn swap_directory(
    root: &Path,
    name: &str,
    new: &Path,
    previous: &Path,
    require_existing: bool,
) -> Result<(), String> {
    let live = target_named(root, name);
    if inspect(previous)?.is_none() {
        match inspect(&live)? {
            Some(metadata) if metadata.is_dir() => {
                std::fs::rename(&live, previous)
                    .map_err(|error| format!("移开旧界面失败: {error}"))?;
                sync_parents(&[&live, previous])
                    .map_err(|error| format!("同步目录失败: {error}"))?;
            }
            Some(_) => return Err(format!("ui/{name} 已不是目录，拒绝替换")),
            None if require_existing => return Err(format!("ui/{name} 已不是目录，拒绝替换")),
            None => {}
        }
    }
    std::fs::rename(new, &live).map_err(|error| format!("放入新界面失败: {error}"))?;
    sync_parents(&[&live, new]).map_err(|error| format!("同步目录失败: {error}"))
}

fn restore_one(root: &Path, name: &str, previous_dir: &str, existed: bool) -> Result<(), String> {
    if previous_dir.is_empty() {
        return Ok(());
    }
    let live = target_named(root, name);
    let previous = Path::new(previous_dir);
    if inspect(previous)?.is_some() {
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
        return sync_parents(&[&live, previous]).map_err(|error| format!("同步目录失败: {error}"));
    }
    if !existed {
        real_ui_directory(root)?;
        match inspect(&live)? {
            Some(metadata) if metadata.is_dir() => {
                std::fs::remove_dir_all(&live)
                    .map_err(|error| format!("移除新安装的 ui/{name} 失败: {error}"))?;
                sync_dir(&root.join("ui")).map_err(|error| format!("同步目录失败: {error}"))?;
            }
            Some(_) => return Err(format!("ui/{name} 已不是目录，拒绝回滚删除")),
            None => {}
        }
    }
    Ok(())
}

/// Puts the release's WebUI package(s) in place (guard, `installing`): `ui/main` and, when
/// staged from `bundled-ui/lg/`, `ui/lg`.
pub(crate) fn install(tx: &Transaction) -> Result<(), String> {
    if tx.webui_archive_path.is_empty() {
        return Ok(());
    }
    let root = Path::new(&tx.config_base_dir);
    let new = Path::new(&tx.webui_new_dir);
    let previous = Path::new(&tx.webui_previous_dir);
    let bytes = std::fs::read(&tx.webui_archive_path)
        .map_err(|error| format!("读取界面资产失败: {error}"))?;
    if super::sha256_hex(&bytes) != tx.webui_sha256 {
        return Err("界面资产的校验值与暂存时不同".to_owned());
    }
    remove_dir_if_exists(new, "界面暂存目录")?;
    let staging = new.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(staging).map_err(|error| format!("创建界面暂存目录失败: {error}"))?;
    extract_tar_gz_tree(&bytes, new, LIMITS)
        .map_err(|reason| format!("解压界面资产失败: {reason}"))?;
    check_tree(new)?;
    let glass_new =
        (!tx.webui_glass_new_dir.is_empty()).then(|| Path::new(&tx.webui_glass_new_dir));
    let extracted_glass = split_bundled_glass(new, glass_new)?;
    if glass_new.is_some() && !extracted_glass {
        return Err("界面资产缺少 bundled-ui/lg".to_owned());
    }
    real_ui_directory(root)?;
    if !same_filesystem(&root.join("ui"), staging).unwrap_or(false) {
        return Err("ui/ 与 update/ 不在同一文件系统，无法原子替换界面".to_owned());
    }
    swap_directory(root, crate::external_ui::MAIN, new, previous, true)?;
    if let Some(glass_new) = glass_new {
        let glass_staging = glass_new.parent().unwrap_or_else(|| Path::new("."));
        if !same_filesystem(&root.join("ui"), glass_staging).unwrap_or(false) {
            return Err("ui/ 与 update/ 不在同一文件系统，无法原子替换界面".to_owned());
        }
        swap_directory(
            root,
            crate::external_ui::GLASS,
            glass_new,
            Path::new(&tx.webui_glass_previous_dir),
            false,
        )?;
    }
    Ok(())
}

/// Puts the old `ui/main` (and `ui/lg`, removing a freshly created one) back on rollback.
/// Idempotent across interruptions.
pub(crate) fn restore(tx: &Transaction) -> Result<(), String> {
    if tx.webui_archive_path.is_empty() {
        return Ok(());
    }
    let root = Path::new(&tx.config_base_dir);
    let mut problems = Vec::new();
    if let Err(error) = restore_one(root, crate::external_ui::MAIN, &tx.webui_previous_dir, true) {
        problems.push(error);
    }
    if !tx.webui_glass_previous_dir.is_empty()
        && let Err(error) = restore_one(
            root,
            crate::external_ui::GLASS,
            &tx.webui_glass_previous_dir,
            tx.webui_glass_existed,
        )
    {
        problems.push(error);
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("；"))
    }
}

/// Normal-startup bridge for hosts upgraded by an older updater (e.g. v0.7.0) or updated by
/// unpacking `atomdns-webui.tar.gz` directly into `ui/main`: atomically moves
/// `ui/main/bundled-ui/lg` to `ui/lg` before the UI registry scans `ui/`.
///
/// Must NOT be called while running as an update-guard trial candidate (rollback safety).
pub(crate) fn promote_bundled_on_start(root: &Path) -> Result<Option<String>, String> {
    if presence(root) != Presence::Directory {
        return Ok(None);
    }
    let main_dir = target(root);
    let bundled_root = main_dir.join(BUNDLED_UI_DIR);
    let Some(root_meta) = inspect(&bundled_root)? else {
        return Ok(None);
    };
    if !root_meta.is_dir() {
        return Ok(None);
    }
    let bundled_lg = bundled_glass_path(&main_dir);
    let Some(lg_meta) = inspect(&bundled_lg)? else {
        let _ = std::fs::remove_dir(&bundled_root);
        return Ok(None);
    };
    if !lg_meta.is_dir() {
        return Err("ui/main/bundled-ui/lg 不是普通目录".to_owned());
    }
    check_tree(&bundled_lg)?;
    real_ui_directory(root)?;
    let live_lg = target_glass(root);
    let note = match presence_glass(root) {
        Presence::Unusable => {
            remove_dir_if_exists(&bundled_lg, "内嵌玻璃界面暂存目录")?;
            let _ = std::fs::remove_dir(&bundled_root);
            let _ = sync_dir(&main_dir);
            "ui/lg 不是普通目录，已保留原状并清理内嵌暂存".to_owned()
        }
        Presence::Absent => {
            std::fs::rename(&bundled_lg, &live_lg)
                .map_err(|error| format!("迁出玻璃界面 ui/lg 失败: {error}"))?;
            let _ = std::fs::remove_dir(&bundled_root);
            let _ = sync_dir(&root.join("ui"));
            let _ = sync_dir(&main_dir);
            "已自动安装随界面包下发的玻璃界面 ui/lg (/lg/)".to_owned()
        }
        Presence::Directory => {
            let scratch = root.join("update");
            std::fs::create_dir_all(&scratch)
                .map_err(|error| format!("创建 update/ 失败: {error}"))?;
            if !same_filesystem(&root.join("ui"), &scratch).unwrap_or(false) {
                return Err("ui/ 与 update/ 不在同一文件系统，无法原子替换玻璃界面".to_owned());
            }
            let previous = scratch.join("bundled-ui-lg-previous");
            remove_dir_if_exists(&previous, "旧玻璃界面暂存目录")?;
            std::fs::rename(&live_lg, &previous)
                .map_err(|error| format!("移开旧玻璃界面失败: {error}"))?;
            if let Err(error) = std::fs::rename(&bundled_lg, &live_lg) {
                let _ = std::fs::rename(&previous, &live_lg);
                return Err(format!("放入新玻璃界面失败: {error}"));
            }
            let _ = sync_dir(&root.join("ui"));
            let _ = sync_dir(&main_dir);
            let _ = sync_dir(&scratch);
            let _ = std::fs::remove_dir_all(&previous);
            let _ = std::fs::remove_dir(&bundled_root);
            "已自动更新随界面包下发的玻璃界面 ui/lg (/lg/)".to_owned()
        }
    };
    Ok(Some(note))
}
