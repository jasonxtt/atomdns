//! Container mode (`ATOMDNS_CONTAINER=1`, set by the Docker image).
//!
//! A container is upgraded by pulling a new image and recreating it, never by replacing the
//! program inside it, so the online updater is switched off (`system.update` reports why) and
//! the image carries what an online update would have brought along: the fresh-install
//! configuration (`config_rust.zip`), the incremental package (`config_rust_up.zip`) and the
//! WebUI (`atomdns-webui.tar.gz`), in `ATOMDNS_CONTAINER_ASSETS` (default `/usr/share/atomdns`).
//!
//! [`prepare`] runs before the configuration is loaded and brings the mounted configuration
//! volume in line with the image:
//!
//! 1. an empty volume (no configuration file) is filled from `config_rust.zip`; a volume that
//!    holds other files but no configuration file is refused, never overwritten;
//! 2. when the volume's applied configuration schema is older than the one this program
//!    requires, the incremental package is applied exactly as the update guard would;
//! 3. when `ui/main` is a real directory and its `ui_version` differs from the image's WebUI,
//!    it is replaced by two renames (a pure backend without `ui/main` stays one).

use std::path::{Path, PathBuf};

use super::archive::{ZipLimits, extract_tar_gz_file, extract_tar_gz_tree, read_zip};
use super::fsio::{durable_write, sync_dir};
use super::package::{self, ConfigState};
use super::webui;
use super::{
    MAX_CONFIG_PACKAGE_BYTES, MAX_WEBUI_ARCHIVE_BYTES, REQUIRED_CONFIG_PACKAGE_ID,
    REQUIRED_CONFIG_SCHEMA,
};

/// Why the updater is off, shown as the capability's reason and in refusals.
pub(crate) const UNSUPPORTED_REASON: &str =
    "容器版请拉取新镜像并重建容器，不支持在 WebUI 内在线更新";

const DEFAULT_ASSETS: &str = "/usr/share/atomdns";
const FRESH_CONFIG: &str = "config_rust.zip";
const INCREMENTAL_CONFIG: &str = "config_rust_up.zip";
const WEBUI_PACKAGE: &str = "atomdns-webui.tar.gz";
const MAX_FRESH_ENTRIES: usize = 4096;

/// Whether this process runs as a container image.
pub(crate) fn enabled() -> bool {
    std::env::var("ATOMDNS_CONTAINER").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn assets_dir() -> PathBuf {
    std::env::var_os("ATOMDNS_CONTAINER_ASSETS")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_ASSETS), PathBuf::from)
}

/// Brings the configuration volume `root` in line with the image. Returns one line per
/// thing it did, for the start-up log.
pub(crate) fn prepare(config: &Path, root: &Path) -> Result<Vec<String>, String> {
    prepare_with(&assets_dir(), config, root)
}

fn read_asset(assets: &Path, name: &str, limit: usize) -> Result<Vec<u8>, String> {
    let path = assets.join(name);
    let bytes =
        std::fs::read(&path).map_err(|error| format!("读取镜像内置的 {name} 失败: {error}"))?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(format!(
            "镜像内置的 {name} 大小不合理（{} 字节）",
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn prepare_with(assets: &Path, config: &Path, root: &Path) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    std::fs::create_dir_all(root)
        .map_err(|error| format!("无法创建配置目录 {}: {error}", root.display()))?;
    let seeded = match std::fs::symlink_metadata(config) {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            seed(assets, config, root)?;
            log.push(format!(
                "空配置目录已用镜像内置配置初始化: {}",
                root.display()
            ));
            true
        }
        Err(error) => return Err(format!("无法检查 {}: {error}", config.display())),
    };
    if let Some(line) = sync_config(assets, root, seeded)? {
        log.push(line);
    }
    if let Some(line) = sync_webui(assets, root)? {
        log.push(line);
    }
    Ok(log)
}

/// A name that is not data: the file system root of a volume mount point may carry one.
fn is_volume_artifact(name: &std::ffi::OsStr) -> bool {
    name == "lost+found"
}

fn seed(assets: &Path, config: &Path, root: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(root)
        .map_err(|error| format!("无法读取配置目录 {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("无法读取配置目录: {error}"))?;
        if !is_volume_artifact(&entry.file_name()) {
            return Err(format!(
                "{} 不是空目录但缺少 {}，拒绝自动初始化（不会覆盖已有文件）",
                root.display(),
                config.display()
            ));
        }
    }
    let bytes = read_asset(assets, FRESH_CONFIG, MAX_CONFIG_PACKAGE_BYTES)?;
    let files = read_zip(
        &bytes,
        ZipLimits {
            max_entries: MAX_FRESH_ENTRIES,
            max_entry_bytes: 64 << 20,
            max_total_bytes: MAX_CONFIG_PACKAGE_BYTES as u64,
        },
    )?;
    for file in files {
        let relative = package::clean_relative(&file.name.replace('\\', "/"))?;
        durable_write(&root.join(&relative), &file.data, 0o644)
            .map_err(|error| format!("写入 {relative:?} 失败: {error}"))?;
    }
    if std::fs::symlink_metadata(config).is_err() {
        return Err(format!(
            "镜像内置的 {FRESH_CONFIG} 没有生成 {}",
            config.display()
        ));
    }
    Ok(())
}

/// Applies the incremental package when the volume's schema is behind the program's.
fn sync_config(assets: &Path, root: &Path, seeded: bool) -> Result<Option<String>, String> {
    let applied = package::load_state(root).applied_schema;
    if applied >= REQUIRED_CONFIG_SCHEMA {
        return Ok(None);
    }
    let bytes = read_asset(assets, INCREMENTAL_CONFIG, MAX_CONFIG_PACKAGE_BYTES)?;
    let package = package::parse(&bytes, REQUIRED_CONFIG_SCHEMA, REQUIRED_CONFIG_PACKAGE_ID)
        .map_err(|reason| format!("镜像内置的配置包不适用于本程序: {reason}"))?;
    let mut backup_dir = String::new();
    if !seeded {
        // The managed files are replaced, so the old ones are kept next to the volume's data.
        let dir = root.join("backup").join(format!(
            "container-schema-{applied}-to-{}-{}",
            REQUIRED_CONFIG_SCHEMA,
            super::now_text().replace([':', ' '], "-")
        ));
        package::backup(root, &dir, &package)
            .map_err(|reason| format!("备份配置失败: {reason}"))?;
        backup_dir = dir.display().to_string();
    }
    let updated = package::apply(root, &package)
        .map_err(|reason| format!("应用镜像内置的配置包失败: {reason}"))?;
    let state = ConfigState {
        applied_schema: package.manifest.config_schema,
        required_schema: package.manifest.config_schema,
        status: "success".to_owned(),
        package_id: package.manifest.package_id.clone(),
        package_sha256: package.sha256.clone(),
        binary_version: crate::build_identity::VERSION.to_owned(),
        files_updated: updated,
        backup_dir: backup_dir.clone(),
        message: format!(
            "容器镜像已将配置升级到 schema {}",
            package.manifest.config_schema
        ),
        updated_at: super::now_text(),
        ..ConfigState::default()
    };
    package::write_state(root, &state).map_err(|error| format!("写入配置状态失败: {error}"))?;
    Ok(Some(if backup_dir.is_empty() {
        format!("配置状态已记为 schema {REQUIRED_CONFIG_SCHEMA}")
    } else {
        format!(
            "配置已从 schema {applied} 升级到 {REQUIRED_CONFIG_SCHEMA}（更新 {updated} 个文件，旧文件备份在 {backup_dir}）"
        )
    }))
}

/// The `ui_version` in a `ui-manifest.json`.
fn ui_version(manifest: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(manifest)
        .ok()?
        .get("ui_version")?
        .as_str()
        .map(str::to_owned)
}

/// Replaces `ui/main` by the image's WebUI when the versions differ.
fn sync_webui(assets: &Path, root: &Path) -> Result<Option<String>, String> {
    if webui::presence(root) != webui::Presence::Directory {
        return Ok(None);
    }
    let archive = read_asset(assets, WEBUI_PACKAGE, MAX_WEBUI_ARCHIVE_BYTES)?;
    let image_manifest = extract_tar_gz_file(
        &archive,
        "ui-manifest.json",
        64 << 10,
        webui::LIMITS.max_total,
    )
    .map_err(|reason| format!("镜像内置的界面包缺少 ui-manifest.json: {reason}"))?;
    let image_version = ui_version(&image_manifest)
        .ok_or_else(|| "镜像内置界面包的 ui-manifest.json 没有 ui_version".to_owned())?;
    let live = webui::target(root);
    let installed = std::fs::read(live.join("ui-manifest.json"))
        .ok()
        .and_then(|bytes| ui_version(&bytes));
    if installed.as_deref() == Some(image_version.as_str()) {
        return Ok(None);
    }
    // Scratch space next to `ui/`, never inside it: every directory of `ui/` is served.
    let scratch = root.join("update");
    let (new, previous) = (
        scratch.join("container-webui-new"),
        scratch.join("container-webui-previous"),
    );
    std::fs::create_dir_all(&scratch).map_err(|error| format!("创建 update/ 失败: {error}"))?;
    for stale in [&new, &previous] {
        match std::fs::remove_dir_all(stale) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("清理 {} 失败: {error}", stale.display())),
        }
    }
    if !webui::same_filesystem(&root.join("ui"), &scratch).unwrap_or(false) {
        return Err("ui/ 与 update/ 不在同一文件系统，无法原子替换界面".to_owned());
    }
    extract_tar_gz_tree(&archive, &new, webui::LIMITS)
        .map_err(|reason| format!("解压界面包失败: {reason}"))?;
    std::fs::rename(&live, &previous).map_err(|error| format!("移开旧界面失败: {error}"))?;
    if let Err(error) = std::fs::rename(&new, &live) {
        // Put the old one back: a missing `ui/main` would silently turn the host into a backend.
        let _ = std::fs::rename(&previous, &live);
        return Err(format!("放入新界面失败: {error}"));
    }
    let _ = sync_dir(&root.join("ui"));
    let _ = sync_dir(&scratch);
    let _ = std::fs::remove_dir_all(&previous);
    Ok(Some(format!(
        "界面已更新: {} → {image_version}",
        installed.as_deref().unwrap_or("未知版本")
    )))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;
    use crate::update::archive::ZipWriter;
    use crate::update::sha256_hex;

    fn temp(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("update-container-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn webui_tgz(version: &str) -> Vec<u8> {
        let manifest = format!(r#"{{"manifest_schema":1,"ui_version":"{version}"}}"#);
        let mut tar = Vec::new();
        for (name, data) in [
            ("index.html", format!("<html>{version}</html>").into_bytes()),
            // Bigger than the scan limit a manifest lookup once had, and before the manifest.
            ("app.js", vec![b'x'; 300 << 10]),
            ("ui-manifest.json", manifest.into_bytes()),
        ] {
            let mut header = [0_u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..107].copy_from_slice(b"0000644");
            header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            header[156] = b'0';
            header[257..262].copy_from_slice(b"ustar");
            tar.extend_from_slice(&header);
            tar.extend_from_slice(&data);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.extend_from_slice(&[0_u8; 1024]);
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    fn config_up(content: &str) -> Vec<u8> {
        let manifest = serde_json::json!({
            "format": 1,
            "channel": "rust",
            "package_id": REQUIRED_CONFIG_PACKAGE_ID,
            "config_schema": REQUIRED_CONFIG_SCHEMA,
            "managed_files": ["config_custom.yaml"],
            "sha256": {"config_custom.yaml": sha256_hex(content.as_bytes())},
        });
        let mut zip = ZipWriter::default();
        zip.add("manifest.json", manifest.to_string().as_bytes())
            .unwrap();
        zip.add("config_custom.yaml", content.as_bytes()).unwrap();
        zip.finish().unwrap()
    }

    fn fresh_zip(config: &str) -> Vec<u8> {
        let mut zip = ZipWriter::default();
        zip.add("config_custom.yaml", config.as_bytes()).unwrap();
        zip.add("webinfo/clientname.json", b"{}").unwrap();
        zip.add("ui/main/index.html", b"<html>old</html>").unwrap();
        zip.add(
            "ui/main/ui-manifest.json",
            br#"{"manifest_schema":1,"ui_version":"v0.0.1"}"#,
        )
        .unwrap();
        zip.finish().unwrap()
    }

    fn assets(name: &str, ui: &str, config: &str) -> PathBuf {
        let dir = temp(name);
        std::fs::write(dir.join(FRESH_CONFIG), fresh_zip("old: true\n")).unwrap();
        std::fs::write(dir.join(INCREMENTAL_CONFIG), config_up(config)).unwrap();
        std::fs::write(dir.join(WEBUI_PACKAGE), webui_tgz(ui)).unwrap();
        dir
    }

    #[test]
    fn an_empty_volume_is_seeded_and_brought_to_the_required_schema_and_ui() {
        let assets = assets("seed-assets", "v9.9.9", "new: true\n");
        let root = temp("seed-root");
        let config = root.join("config_custom.yaml");
        let log = prepare_with(&assets, &config, &root).unwrap();
        assert_eq!(log.len(), 3, "{log:?}");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "new: true\n");
        assert!(root.join("webinfo/clientname.json").is_file());
        assert_eq!(
            package::load_state(&root).applied_schema,
            REQUIRED_CONFIG_SCHEMA
        );
        assert_eq!(
            ui_version(&std::fs::read(root.join("ui/main/ui-manifest.json")).unwrap()).as_deref(),
            Some("v9.9.9")
        );
        assert!(
            !root.join("backup").exists(),
            "a fresh seed needs no backup"
        );
        // A second run changes nothing.
        assert!(prepare_with(&assets, &config, &root).unwrap().is_empty());
    }

    #[test]
    fn a_volume_with_other_files_but_no_configuration_is_refused() {
        let assets = assets("refuse-assets", "v9.9.9", "new: true\n");
        let root = temp("refuse-root");
        std::fs::write(root.join("notes.txt"), "mine").unwrap();
        let error = prepare_with(&assets, &root.join("config_custom.yaml"), &root).unwrap_err();
        assert!(error.contains("拒绝自动初始化"), "{error}");
        assert!(!root.join("config_custom.yaml").exists());
    }

    #[test]
    fn an_old_volume_gets_the_package_with_a_backup_and_keeps_unmanaged_files() {
        let assets = assets("old-assets", "v9.9.9", "new: true\n");
        let root = temp("old-root");
        std::fs::write(root.join("config_custom.yaml"), "mine: true\n").unwrap();
        std::fs::create_dir_all(root.join("webinfo")).unwrap();
        std::fs::write(root.join("webinfo/settings.json"), "{\"version\":1}").unwrap();
        let log = prepare_with(&assets, &root.join("config_custom.yaml"), &root).unwrap();
        assert_eq!(log.len(), 1, "{log:?}");
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "new: true\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("webinfo/settings.json")).unwrap(),
            "{\"version\":1}"
        );
        let backups: Vec<_> = std::fs::read_dir(root.join("backup")).unwrap().collect();
        assert_eq!(backups.len(), 1);
        // No `ui/main` on the volume: it stays a pure backend.
        assert!(!root.join("ui").exists());
    }

    #[test]
    fn a_stale_ui_is_replaced_and_a_current_one_is_left_alone() {
        let assets = assets("ui-assets", "v2.0.0", "new: true\n");
        let root = temp("ui-root");
        let config = root.join("config_custom.yaml");
        prepare_with(&assets, &config, &root).unwrap();
        std::fs::write(root.join("ui/main/index.html"), "edited").unwrap();
        // Same version as the image: untouched, even though the file differs.
        assert!(prepare_with(&assets, &config, &root).unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("ui/main/index.html")).unwrap(),
            "edited"
        );
        // A newer image replaces it, and leaves no scratch directory behind in `ui/`.
        std::fs::write(assets.join(WEBUI_PACKAGE), webui_tgz("v2.1.0")).unwrap();
        let log = prepare_with(&assets, &config, &root).unwrap();
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(
            std::fs::read_to_string(root.join("ui/main/index.html"))
                .unwrap()
                .contains("v2.1.0")
        );
        let names: Vec<_> = std::fs::read_dir(root.join("ui")).unwrap().collect();
        assert_eq!(names.len(), 1);
    }
}
