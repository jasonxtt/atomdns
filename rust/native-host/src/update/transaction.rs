//! The update transaction: its file, its states, staging, rollback and startup recovery.
//!
//! Everything here is built so that an interruption at any point leaves either the old
//! files, the new files, or a state (`installing`/`testing`) from which [`rollback`]
//! restores the old ones. [`rollback`] only ever copies fixed content to fixed places, so
//! it can be repeated after being interrupted itself. See `guard.rs` for the process that
//! drives the states and `docs/rust/contracts/native-update.md` for the argument.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::fsio::{check_writable_dir, durable_copy, durable_write, sync_dir};
use super::package::{self, Package};

pub(crate) const FORMAT: u32 = 1;
/// A transaction that was staged but never started is garbage after this long.
const STAGED_STALE_SECS: u64 = 24 * 3600;
/// A staged transaction younger than this still blocks a new one.
const STAGED_BUSY_SECS: u64 = 10 * 60;
const INSTALLED_FILE: &str = "installed.json";
pub(crate) const MARKER_ENV: &str = "MOSDNS_UPDATE_GUARD_TXN";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Transaction {
    pub(crate) format: u32,
    /// `"binary"` (replace the program, maybe the configuration) or `"config"`.
    pub(crate) kind: String,
    /// `staged` → `installing` → `testing` → `verified`, or `rolled_back`.
    pub(crate) status: String,
    pub(crate) target_version: String,
    pub(crate) previous_version: String,
    pub(crate) asset_signature: String,
    pub(crate) required_config_schema: u32,
    pub(crate) config_package_id: String,
    pub(crate) config_package_sha256: String,
    pub(crate) config_base_dir: String,
    pub(crate) config_path: String,
    pub(crate) executable_path: String,
    pub(crate) candidate_path: String,
    pub(crate) candidate_sha256: String,
    pub(crate) previous_binary_path: String,
    pub(crate) previous_sha256: String,
    /// Empty when there is no configuration change.
    pub(crate) config_package_path: String,
    pub(crate) backup_dir: String,
    pub(crate) health_url: String,
    pub(crate) health_timeout_secs: u64,
    pub(crate) original_args: Vec<String>,
    pub(crate) created_at: u64,
    #[serde(default)]
    pub(crate) last_error: String,
    /// The staged `rtdns-webui.tar.gz`; empty when `ui/main` is not replaced.
    #[serde(default)]
    pub(crate) webui_archive_path: String,
    #[serde(default)]
    pub(crate) webui_sha256: String,
    /// Where the guard unpacks the new WebUI before moving it to `ui/main`.
    #[serde(default)]
    pub(crate) webui_new_dir: String,
    /// Where the old `ui/main` waits until the update is verified or rolled back.
    #[serde(default)]
    pub(crate) webui_previous_dir: String,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn update_dir(root: &Path) -> PathBuf {
    root.join("update")
}

pub(crate) fn file_of(dir: &Path) -> PathBuf {
    dir.join("transaction.json")
}

pub(crate) fn read(path: &Path) -> Result<Transaction, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let tx: Transaction =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if tx.format != FORMAT {
        return Err(format!(
            "unsupported update transaction format {}",
            tx.format
        ));
    }
    Ok(tx)
}

pub(crate) fn write(path: &Path, tx: &Transaction) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(tx).map_err(|error| error.to_string())?;
    durable_write(path, &bytes, 0o600).map_err(|error| format!("write {}: {error}", path.display()))
}

/// Every readable transaction under `<root>/update/`, oldest first.
pub(crate) fn list(root: &Path) -> Vec<(PathBuf, Transaction)> {
    let Ok(entries) = std::fs::read_dir(update_dir(root)) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, Transaction)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("txn-"))
        .filter_map(|entry| {
            let dir = entry.path();
            let tx = read(&file_of(&dir)).ok()?;
            Some((dir, tx))
        })
        .collect();
    found.sort_by_key(|(_, tx)| tx.created_at);
    found
}

/// The transaction that makes `pending_restart` true / blocks a new one, if any.
pub(crate) fn pending(root: &Path) -> Option<Transaction> {
    let now = now_secs();
    list(root)
        .into_iter()
        .map(|(_, tx)| tx)
        .rfind(|tx| match tx.status.as_str() {
            "installing" | "testing" => true,
            "staged" => now.saturating_sub(tx.created_at) < STAGED_BUSY_SECS,
            _ => false,
        })
}

/// The message of the most recent rolled-back transaction.
pub(crate) fn latest_rollback(root: &Path) -> Option<String> {
    list(root)
        .into_iter()
        .map(|(_, tx)| tx)
        .rfind(|tx| tx.status == "rolled_back")
        .map(|tx| {
            if tx.last_error.is_empty() {
                "候选版本未通过健康检查".to_owned()
            } else {
                tx.last_error
            }
        })
}

#[derive(Serialize, Deserialize)]
struct Installed {
    version: String,
    asset_signature: String,
    installed_at: u64,
}

/// The signature of the build that was installed through the updater (empty when none).
pub(crate) fn installed_signature(root: &Path) -> String {
    std::fs::read(update_dir(root).join(INSTALLED_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Installed>(&bytes).ok())
        .map(|installed| installed.asset_signature)
        .unwrap_or_default()
}

pub(crate) fn write_installed(root: &Path, version: &str, signature: &str) -> Result<(), String> {
    let bytes = serde_json::to_vec(&Installed {
        version: version.to_owned(),
        asset_signature: signature.to_owned(),
        installed_at: now_secs(),
    })
    .map_err(|error| error.to_string())?;
    durable_write(&update_dir(root).join(INSTALLED_FILE), &bytes, 0o600)
        .map_err(|error| format!("write installed.json: {error}"))
}

/// Removes finished transactions: when one was verified, every verified and rolled-back
/// one; stale staged ones; directories that never got a transaction file.
pub(crate) fn cleanup(root: &Path) {
    let all = list(root);
    let verified = all.iter().any(|(_, tx)| tx.status == "verified");
    let now = now_secs();
    for (dir, tx) in &all {
        let remove = match tx.status.as_str() {
            "verified" | "rolled_back" => verified,
            "staged" | "abandoned" => now.saturating_sub(tx.created_at) > STAGED_STALE_SECS,
            _ => false,
        };
        if remove {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    if let Ok(entries) = std::fs::read_dir(update_dir(root)) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with("txn-")
                && path.is_dir()
                && !file_of(&path).exists()
                && path
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 3600)
            {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}

/// What staging needs.
pub(crate) struct StageInput<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config_path: &'a Path,
    /// The running program and its `argv`, exactly as started.
    pub(crate) executable: &'a Path,
    pub(crate) args: Vec<String>,
    pub(crate) health_url: String,
    pub(crate) health_timeout_secs: u64,
    pub(crate) kind: &'a str,
    pub(crate) previous_version: &'a str,
    pub(crate) target_version: &'a str,
    pub(crate) asset_signature: &'a str,
    /// The new program (`kind == "binary"`).
    pub(crate) candidate: Option<&'a [u8]>,
    /// The configuration package and its original bytes, when the update carries one.
    pub(crate) package: Option<(&'a Package, &'a [u8])>,
    /// The release's `rtdns-webui.tar.gz`, when `ui/main` is replaced.
    pub(crate) webui: Option<&'a [u8]>,
}

/// What a candidate must pass before the service is touched.
pub(crate) type Preflight<'a> = &'a dyn Fn(&Path, Option<&Path>) -> Result<(), String>;

/// Creates the transaction directory with everything the guard will need — the running
/// program's copy, the candidate, the package, and a complete backup of the configuration
/// files the package touches — runs `preflight` on the candidate (the program to run, and
/// the preview root of the new configuration, if any) and only then writes
/// `transaction.json` with status `staged`. Nothing in the live installation changes. On
/// any failure the directory is removed.
pub(crate) fn stage(
    input: &StageInput<'_>,
    preflight: Preflight<'_>,
) -> Result<(PathBuf, Transaction), String> {
    if input.kind == "binary" {
        // Installing and rolling back both replace the program in its own directory; a
        // directory that cannot take a new file would make the rollback impossible.
        let program_dir = input
            .executable
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        check_writable_dir(program_dir).map_err(|error| {
            format!(
                "程序所在目录 {} 不可写，无法安全替换/回滚: {error}",
                program_dir.display()
            )
        })?;
    }
    let base = update_dir(input.root);
    std::fs::create_dir_all(&base).map_err(|error| format!("create update directory: {error}"))?;
    let dir = base.join(format!("txn-{}-{:08x}", now_secs(), rand_suffix()));
    std::fs::create_dir(&dir).map_err(|error| format!("create transaction directory: {error}"))?;
    let result = stage_into(&dir, input, preflight).and_then(|tx| {
        // The entries leading to the transaction must be durable before the live files are
        // touched, or a power loss could leave new files and no record of the backup.
        sync_dir(&base)
            .and_then(|()| sync_dir(input.root))
            .map_err(|error| format!("sync update directory: {error}"))?;
        Ok(tx)
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result.map(|tx| (file_of(&dir), tx))
}

fn rand_suffix() -> u32 {
    let mut bytes = [0_u8; 4];
    let _ = getrandom::fill(&mut bytes);
    u32::from_le_bytes(bytes)
}

fn stage_into(
    dir: &Path,
    input: &StageInput<'_>,
    preflight: Preflight<'_>,
) -> Result<Transaction, String> {
    let previous = dir.join("mosdns.previous");
    durable_copy(input.executable, &previous, 0o755)
        .map_err(|error| format!("备份当前程序失败: {error}"))?;
    let previous_sha256 = super::file_sha256(&previous).map_err(|error| error.to_string())?;
    let (candidate_path, candidate_sha256) = match input.candidate {
        Some(bytes) => {
            let path = dir.join("mosdns.candidate");
            durable_write(&path, bytes, 0o755)
                .map_err(|error| format!("暂存候选程序失败: {error}"))?;
            (path.display().to_string(), super::sha256_hex(bytes))
        }
        None => (String::new(), String::new()),
    };
    let candidate_program = if input.candidate.is_some() {
        PathBuf::from(&candidate_path)
    } else {
        previous.clone()
    };
    let (package_path, package_sha, backup_dir) = match input.package {
        Some((package, bytes)) => {
            let path = dir.join("config_up.zip");
            durable_write(&path, bytes, 0o600)
                .map_err(|error| format!("暂存配置包失败: {error}"))?;
            let backup_dir = dir.join("config-backup");
            package::backup(input.root, &backup_dir, package)?;
            (
                path.display().to_string(),
                package.sha256.clone(),
                backup_dir.display().to_string(),
            )
        }
        None => (String::new(), String::new(), String::new()),
    };
    let (webui_archive_path, webui_sha256, webui_new_dir, webui_previous_dir) = match input.webui {
        Some(bytes) => {
            let webui = dir.join("webui");
            std::fs::create_dir_all(&webui)
                .map_err(|error| format!("创建界面暂存目录失败: {error}"))?;
            let ui = super::webui::target(input.root)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            if !super::webui::same_filesystem(&ui, &webui).unwrap_or(false) {
                return Err("ui/ 与 update/ 不在同一文件系统，无法原子替换界面".to_owned());
            }
            super::webui::check_archive(bytes, &webui.join("check"))?;
            let path = dir.join(super::WEBUI_ASSET);
            durable_write(&path, bytes, 0o600)
                .map_err(|error| format!("暂存界面资产失败: {error}"))?;
            (
                path.display().to_string(),
                super::sha256_hex(bytes),
                webui.join("new").display().to_string(),
                webui.join("previous").display().to_string(),
            )
        }
        None => Default::default(),
    };
    // Pre-flight: run the candidate against the new configuration before anything is touched.
    match input.package {
        Some((package, _)) => {
            let preview = dir.join("preview");
            package::build_preview(input.root, package, &preview)?;
            let result = preflight(&candidate_program, Some(&preview));
            let _ = std::fs::remove_dir_all(&preview);
            result?;
        }
        None => preflight(&candidate_program, None)?,
    }
    let tx = Transaction {
        format: FORMAT,
        kind: input.kind.to_owned(),
        status: "staged".to_owned(),
        target_version: input.target_version.to_owned(),
        previous_version: input.previous_version.to_owned(),
        asset_signature: input.asset_signature.to_owned(),
        required_config_schema: input.package.map_or(0, |(p, _)| p.manifest.config_schema),
        config_package_id: input
            .package
            .map(|(p, _)| p.manifest.package_id.clone())
            .unwrap_or_default(),
        config_package_sha256: package_sha,
        config_base_dir: input.root.display().to_string(),
        config_path: input.config_path.display().to_string(),
        executable_path: input.executable.display().to_string(),
        candidate_path,
        candidate_sha256,
        previous_binary_path: previous.display().to_string(),
        previous_sha256,
        config_package_path: package_path,
        backup_dir,
        health_url: input.health_url.clone(),
        health_timeout_secs: input.health_timeout_secs,
        original_args: input.args.clone(),
        created_at: now_secs(),
        last_error: String::new(),
        webui_archive_path,
        webui_sha256,
        webui_new_dir,
        webui_previous_dir,
    };
    write(&file_of(dir), &tx)?;
    sync_dir(dir).map_err(|error| error.to_string())?;
    Ok(tx)
}

/// Whether `path` is inside `<root>/update/` (a transaction never names a file elsewhere).
fn inside_update_dir(root: &Path, path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    Path::new(path).starts_with(update_dir(root)) && !path.split('/').any(|part| part == "..")
}

/// Checks that the paths in a transaction file are the ones this program would have
/// written (so a doctored file cannot point the guard at other files).
pub(crate) fn validate_paths(tx: &Transaction) -> Result<(), String> {
    let root = Path::new(&tx.config_base_dir);
    if tx.original_args.is_empty() || tx.executable_path.is_empty() {
        return Err("transaction has no program or arguments".to_owned());
    }
    for path in [
        &tx.candidate_path,
        &tx.previous_binary_path,
        &tx.config_package_path,
        &tx.backup_dir,
        &tx.webui_archive_path,
        &tx.webui_new_dir,
        &tx.webui_previous_dir,
    ] {
        if !inside_update_dir(root, path) {
            return Err(format!(
                "transaction path {path:?} is outside the update directory"
            ));
        }
    }
    if tx.previous_binary_path.is_empty() {
        return Err("transaction has no previous program".to_owned());
    }
    // The WebUI paths are this transaction's own fixed ones (the guard removes and renames
    // whole directories there): `<txn>/rtdns-webui.tar.gz`, `<txn>/webui/{new,previous}`,
    // where `<txn>` is the `update/txn-*` directory of the saved program.
    let webui = [
        &tx.webui_archive_path,
        &tx.webui_new_dir,
        &tx.webui_previous_dir,
    ];
    if webui.iter().any(|path| !path.is_empty()) {
        let txn = Path::new(&tx.previous_binary_path)
            .parent()
            .filter(|dir| {
                dir.parent() == Some(update_dir(root).as_path())
                    && dir
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("txn-"))
            })
            .ok_or("transaction has no transaction directory for the WebUI")?;
        let expected = [
            txn.join(super::WEBUI_ASSET),
            txn.join("webui").join("new"),
            txn.join("webui").join("previous"),
        ];
        if webui
            .iter()
            .zip(&expected)
            .any(|(path, expected)| Path::new(path.as_str()) != expected.as_path())
        {
            return Err("transaction WebUI paths are not the transaction's own".to_owned());
        }
    }
    Ok(())
}

/// Puts the configuration, the WebUI (when the update replaced it) and (for `binary`)
/// the program back. Idempotent; writes the `rolled_back` status only after all are
/// restored.
pub(crate) fn rollback(path: &Path, tx: &mut Transaction, cause: &str) -> Result<(), String> {
    validate_paths(tx)?;
    let root = Path::new(&tx.config_base_dir);
    let mut problems = Vec::new();
    if !tx.backup_dir.is_empty() {
        if let Err(error) = package::restore(root, Path::new(&tx.backup_dir)) {
            problems.push(format!("恢复旧配置失败: {error}"));
        }
    }
    if let Err(error) = super::webui::restore(tx) {
        problems.push(format!("恢复旧界面失败: {error}"));
    }
    if tx.kind == "binary" {
        let previous = Path::new(&tx.previous_binary_path);
        match super::file_sha256(previous) {
            Ok(actual) if actual == tx.previous_sha256 => {
                let live = Path::new(&tx.executable_path);
                // Nothing to restore when the old program is still (or again) in place.
                let in_place = super::file_sha256(live).is_ok_and(|sha| sha == actual);
                if !in_place {
                    if let Err(error) = durable_copy(previous, live, 0o755) {
                        problems.push(format!("恢复旧程序失败: {error}"));
                    }
                }
            }
            Ok(_) => problems.push("旧程序备份的校验值不符，拒绝恢复".to_owned()),
            Err(error) => problems.push(format!("读取旧程序备份失败: {error}")),
        }
    }
    if !problems.is_empty() {
        // Leave the state as it was: recovery will try again at the next start.
        return Err(format!("{cause}；{}", problems.join("；")));
    }
    tx.status = "rolled_back".to_owned();
    tx.last_error = cause.to_owned();
    write(path, tx)
}

/// Closes a transaction that never left `staged` (its guard could not be started) as rolled
/// back with `reason`; nothing live was changed, so there is nothing to restore.
pub(crate) fn abandon_staged(path: &Path, reason: &str) {
    if let Ok(mut tx) = read(path) {
        if tx.status == "staged" {
            tx.status = "rolled_back".to_owned();
            tx.last_error = reason.to_owned();
            let _ = write(path, &tx);
        }
    }
}

/// The result of the start-up recovery.
pub(crate) struct Recovered {
    pub(crate) executable: String,
    pub(crate) args: Vec<String>,
}

/// Start-up recovery (before the configuration is loaded): rolls back any `installing` or
/// `testing` transaction, and cleans up. Not run by a candidate the guard started.
/// Returns the program to run again when something was rolled back.
pub(crate) fn recover_on_start(root: &Path) -> Result<Option<Recovered>, String> {
    if std::env::var_os(MARKER_ENV).is_some() {
        return Ok(None);
    }
    let mut recovered = None;
    for (dir, mut tx) in list(root).into_iter().rev() {
        if tx.status != "installing" && tx.status != "testing" {
            continue;
        }
        rollback(
            &file_of(&dir),
            &mut tx,
            "检测到未完成的更新，已在启动前自动回滚",
        )?;
        recovered = Some(Recovered {
            executable: tx.executable_path.clone(),
            args: tx.original_args.clone(),
        });
        break;
    }
    if recovered.is_none() {
        for (dir, mut tx) in list(root) {
            if tx.status == "staged" && now_secs().saturating_sub(tx.created_at) > STAGED_STALE_SECS
            {
                tx.status = "abandoned".to_owned();
                let _ = write(&file_of(&dir), &tx);
            }
        }
        cleanup(root);
    }
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::archive::ZipWriter;
    use crate::update::sha256_hex;

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("update-txn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn package_bytes() -> Vec<u8> {
        let content = "log: {level: info}\n";
        let manifest = serde_json::json!({
            "format": 1, "channel": "rust", "package_id": "rust-config-schema-1", "config_schema": 1,
            "managed_files": ["config_custom.yaml"],
            "sha256": {"config_custom.yaml": sha256_hex(content.as_bytes())},
        });
        let mut zip = ZipWriter::default();
        zip.add("manifest.json", manifest.to_string().as_bytes())
            .unwrap();
        zip.add("config_custom.yaml", content.as_bytes()).unwrap();
        zip.finish().unwrap()
    }

    struct Fixture {
        root: PathBuf,
        exe: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let root = temp(name);
        std::fs::write(root.join("config_custom.yaml"), "old config\n").unwrap();
        let exe = root.join("mosdns-live");
        std::fs::write(&exe, b"OLD-PROGRAM").unwrap();
        std::fs::set_permissions(&exe, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        Fixture { root, exe }
    }

    fn stage_for(
        fixture: &Fixture,
        kind: &str,
        with_package: bool,
        preflight: Preflight<'_>,
    ) -> Result<(PathBuf, Transaction), String> {
        let bytes = package_bytes();
        let package = package::parse(&bytes, 1, "rust-config-schema-1").unwrap();
        stage(
            &StageInput {
                root: &fixture.root,
                config_path: &fixture.root.join("config_custom.yaml"),
                executable: &fixture.exe,
                args: vec![
                    "mosdns".into(),
                    "start".into(),
                    "-d".into(),
                    fixture.root.display().to_string(),
                ],
                health_url: "http://127.0.0.1:9/api/v1/system/health".into(),
                health_timeout_secs: 5,
                kind,
                previous_version: "v0.1.0",
                target_version: "v0.2.0",
                asset_signature: "asset:abc",
                candidate: (kind == "binary").then_some(b"NEW-PROGRAM".as_slice()),
                package: with_package.then_some((&package, bytes.as_slice())),
                webui: None,
            },
            preflight,
        )
    }

    #[test]
    fn staging_writes_everything_the_guard_needs_and_changes_nothing_live() {
        let fixture = fixture("stage");
        let (path, tx) = stage_for(&fixture, "binary", true, &|_, preview| {
            let preview = preview.expect("a package is previewed");
            assert_eq!(
                std::fs::read_to_string(preview.join("config_custom.yaml")).unwrap(),
                "log: {level: info}\n"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(tx.status, "staged");
        assert_eq!(read(&path).unwrap(), tx);
        let dir = path.parent().unwrap();
        assert_eq!(
            std::fs::read(dir.join("mosdns.previous")).unwrap(),
            b"OLD-PROGRAM"
        );
        assert_eq!(
            std::fs::read(dir.join("mosdns.candidate")).unwrap(),
            b"NEW-PROGRAM"
        );
        assert!(
            dir.join("config_up.zip").exists()
                && dir.join("config-backup/backup_manifest.json").exists()
        );
        assert_eq!(tx.previous_sha256, sha256_hex(b"OLD-PROGRAM"));
        assert_eq!(tx.candidate_sha256, sha256_hex(b"NEW-PROGRAM"));
        // The live installation is untouched.
        assert_eq!(std::fs::read(&fixture.exe).unwrap(), b"OLD-PROGRAM");
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("config_custom.yaml")).unwrap(),
            "old config\n"
        );
        assert!(!dir.join("preview").exists(), "the preview is removed");
        validate_paths(&tx).unwrap();
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_failed_preflight_leaves_no_transaction_behind() {
        let fixture = fixture("preflight");
        let error = stage_for(&fixture, "binary", true, &|_, _| {
            Err("新配置不通过".to_owned())
        })
        .unwrap_err();
        assert!(error.contains("新配置不通过"));
        assert!(list(&fixture.root).is_empty());
        let leftovers: Vec<_> = std::fs::read_dir(update_dir(&fixture.root))
            .unwrap()
            .collect();
        assert!(leftovers.is_empty(), "the staging directory was removed");
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    /// Plays the guard's destructive steps by hand: package applied, program replaced.
    fn install(fixture: &Fixture, path: &Path, tx: &mut Transaction, status: &str) {
        tx.status = status.to_owned();
        write(path, tx).unwrap();
        let bytes = package_bytes();
        let package = package::parse(&bytes, 1, "rust-config-schema-1").unwrap();
        package::apply(&fixture.root, &package).unwrap();
        package::write_state(
            &fixture.root,
            &package::ConfigState {
                applied_schema: 1,
                status: "success".into(),
                ..Default::default()
            },
        )
        .unwrap();
        durable_copy(Path::new(&tx.candidate_path), &fixture.exe, 0o755).unwrap();
    }

    #[test]
    fn a_rollback_restores_program_and_configuration_and_is_repeatable() {
        let fixture = fixture("rollback");
        let (path, mut tx) = stage_for(&fixture, "binary", true, &|_, _| Ok(())).unwrap();
        install(&fixture, &path, &mut tx, "testing");
        assert_eq!(std::fs::read(&fixture.exe).unwrap(), b"NEW-PROGRAM");
        rollback(&path, &mut tx, "候选未通过").unwrap();
        assert_eq!(std::fs::read(&fixture.exe).unwrap(), b"OLD-PROGRAM");
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("config_custom.yaml")).unwrap(),
            "old config\n"
        );
        assert_eq!(package::load_state(&fixture.root).applied_schema, 0);
        assert_eq!(
            (read(&path).unwrap().status, read(&path).unwrap().last_error),
            ("rolled_back".to_owned(), "候选未通过".to_owned())
        );
        assert_eq!(
            latest_rollback(&fixture.root).as_deref(),
            Some("候选未通过")
        );
        // Running it again (an interrupted rollback re-run) changes nothing.
        rollback(&path, &mut tx, "候选未通过").unwrap();
        assert_eq!(std::fs::read(&fixture.exe).unwrap(), b"OLD-PROGRAM");
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_program_directory_that_takes_no_new_file_is_refused_before_anything_is_staged() {
        let mut fixture = fixture("unwritable");
        fixture.exe = fixture.root.join("no-such-dir/mosdns");
        let error = stage_for(&fixture, "binary", true, &|_, _| Ok(())).unwrap_err();
        assert!(error.contains("不可写"), "{error}");
        assert!(list(&fixture.root).is_empty());
        assert!(!update_dir(&fixture.root).exists(), "nothing was staged");
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_rollback_leaves_the_program_alone_when_the_old_one_is_already_in_place() {
        use std::os::unix::fs::MetadataExt;
        let fixture = fixture("in-place");
        let (path, mut tx) = stage_for(&fixture, "binary", true, &|_, _| Ok(())).unwrap();
        // The guard stopped before the program was swapped: only the configuration changed.
        tx.status = "installing".to_owned();
        write(&path, &tx).unwrap();
        let package = package::parse(&package_bytes(), 1, "rust-config-schema-1").unwrap();
        package::apply(&fixture.root, &package).unwrap();
        let before = std::fs::metadata(&fixture.exe).unwrap().ino();
        rollback(&path, &mut tx, "中断").unwrap();
        assert_eq!(
            std::fs::metadata(&fixture.exe).unwrap().ino(),
            before,
            "the live program was not rewritten"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("config_custom.yaml")).unwrap(),
            "old config\n"
        );
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_rollback_refuses_a_corrupted_program_backup_and_keeps_the_state_for_recovery() {
        let fixture = fixture("corrupt");
        let (path, mut tx) = stage_for(&fixture, "binary", false, &|_, _| Ok(())).unwrap();
        install(&fixture, &path, &mut tx, "testing");
        std::fs::write(&tx.previous_binary_path, b"TAMPERED").unwrap();
        let error = rollback(&path, &mut tx, "x").unwrap_err();
        assert!(error.contains("校验值不符"), "{error}");
        assert_eq!(
            read(&path).unwrap().status,
            "testing",
            "recovery can still try again"
        );
        assert_eq!(
            std::fs::read(&fixture.exe).unwrap(),
            b"NEW-PROGRAM",
            "the live program is not overwritten with the tampered copy"
        );
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_config_transaction_never_replaces_the_program() {
        let fixture = fixture("config-kind");
        let (path, mut tx) = stage_for(&fixture, "config", true, &|program, _| {
            assert!(
                program.ends_with("mosdns.previous"),
                "the running program is the candidate"
            );
            Ok(())
        })
        .unwrap();
        assert!(tx.candidate_path.is_empty());
        tx.status = "testing".to_owned();
        write(&path, &tx).unwrap();
        let package = package::parse(&package_bytes(), 1, "rust-config-schema-1").unwrap();
        package::apply(&fixture.root, &package).unwrap();
        std::fs::write(&fixture.exe, b"STILL-RUNNING").unwrap();
        rollback(&path, &mut tx, "配置启动失败").unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("config_custom.yaml")).unwrap(),
            "old config\n"
        );
        assert_eq!(
            std::fs::read(&fixture.exe).unwrap(),
            b"STILL-RUNNING",
            "the program is left alone"
        );
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn recovery_rolls_back_installing_and_testing_but_not_staged_or_verified() {
        for status in ["installing", "testing"] {
            let fixture = fixture(&format!("recover-{status}"));
            let (path, mut tx) = stage_for(&fixture, "binary", true, &|_, _| Ok(())).unwrap();
            install(&fixture, &path, &mut tx, status);
            let recovered = recover_on_start(&fixture.root)
                .unwrap()
                .expect("rolled back");
            assert_eq!(recovered.executable, fixture.exe.display().to_string());
            assert_eq!(recovered.args[1], "start");
            assert_eq!(
                std::fs::read(&fixture.exe).unwrap(),
                b"OLD-PROGRAM",
                "{status}"
            );
            assert_eq!(
                std::fs::read_to_string(fixture.root.join("config_custom.yaml")).unwrap(),
                "old config\n"
            );
            assert_eq!(read(&path).unwrap().status, "rolled_back");
            // Nothing left to recover the second time.
            assert!(recover_on_start(&fixture.root).unwrap().is_none());
            std::fs::remove_dir_all(fixture.root).unwrap();
        }
        let fixture = fixture("recover-staged");
        let (path, mut tx) = stage_for(&fixture, "binary", true, &|_, _| Ok(())).unwrap();
        assert!(recover_on_start(&fixture.root).unwrap().is_none());
        assert_eq!(
            read(&path).unwrap().status,
            "staged",
            "a fresh staged transaction is left alone"
        );
        assert!(
            pending(&fixture.root).is_some(),
            "and still blocks another one"
        );
        tx.status = "verified".to_owned();
        write(&path, &tx).unwrap();
        assert!(recover_on_start(&fixture.root).unwrap().is_none());
        assert!(!path.exists(), "verified transactions are cleaned up");
        assert!(pending(&fixture.root).is_none());
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_guard_that_could_not_start_closes_its_staged_transaction_and_only_that() {
        let fixture = fixture("abandon");
        let (path, tx) = stage_for(&fixture, "binary", false, &|_, _| Ok(())).unwrap();
        abandon_staged(&path, "无法启动更新守护: nope");
        let closed = read(&path).unwrap();
        assert_eq!(
            (closed.status.as_str(), closed.last_error.as_str()),
            ("rolled_back", "无法启动更新守护: nope")
        );
        assert_eq!(std::fs::read(&fixture.exe).unwrap(), b"OLD-PROGRAM");
        // A transaction that already moved on is not touched.
        let mut testing = tx.clone();
        testing.status = "testing".to_owned();
        write(&path, &testing).unwrap();
        abandon_staged(&path, "late");
        assert_eq!(read(&path).unwrap().status, "testing");
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_doctored_transaction_cannot_point_outside_the_update_directory() {
        let fixture = fixture("doctored");
        let (_path, tx) = stage_for(&fixture, "binary", true, &|_, _| Ok(())).unwrap();
        for edit in [
            |tx: &mut Transaction| tx.previous_binary_path = "/etc/passwd".to_owned(),
            |tx: &mut Transaction| tx.candidate_path = "/bin/sh".to_owned(),
            |tx: &mut Transaction| tx.backup_dir = "/root".to_owned(),
            |tx: &mut Transaction| tx.config_package_path = "/tmp/x.zip".to_owned(),
            |tx: &mut Transaction| tx.original_args.clear(),
        ] {
            let mut doctored = tx.clone();
            edit(&mut doctored);
            assert!(validate_paths(&doctored).is_err());
        }
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn cleanup_keeps_the_latest_rollback_until_a_later_success() {
        let fixture = fixture("cleanup");
        let (path, mut tx) = stage_for(&fixture, "binary", false, &|_, _| Ok(())).unwrap();
        install(&fixture, &path, &mut tx, "testing");
        rollback(&path, &mut tx, "failed once").unwrap();
        cleanup(&fixture.root);
        assert!(path.exists(), "a rollback stays visible");
        assert_eq!(
            latest_rollback(&fixture.root).as_deref(),
            Some("failed once")
        );
        let (second, mut tx2) = stage_for(&fixture, "binary", false, &|_, _| Ok(())).unwrap();
        tx2.status = "verified".to_owned();
        write(&second, &tx2).unwrap();
        cleanup(&fixture.root);
        assert!(!path.exists() && !second.exists());
        assert_eq!(installed_signature(&fixture.root), "");
        write_installed(&fixture.root, "v0.2.0", "asset:abc").unwrap();
        assert_eq!(installed_signature(&fixture.root), "asset:abc");
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    fn webui_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write as _;
        let mut tar = Vec::new();
        for (name, data) in entries {
            let mut header = [0_u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..107].copy_from_slice(b"0000644");
            header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            header[156] = b'0';
            header[257..262].copy_from_slice(b"ustar");
            tar.extend_from_slice(&header);
            tar.extend_from_slice(data);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.extend_from_slice(&[0_u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    fn good_webui(marker: &str) -> Vec<u8> {
        webui_tar(&[
            ("index.html", marker.as_bytes()),
            (
                "ui-manifest.json",
                br#"{"manifest_schema":1,"ui_version":"v0.4.0","api_level":1}"#,
            ),
            ("chunks/a.js", b"js"),
        ])
    }

    fn stage_webui(fixture: &Fixture, webui: &[u8]) -> Result<(PathBuf, Transaction), String> {
        stage(
            &StageInput {
                root: &fixture.root,
                config_path: &fixture.root.join("config_custom.yaml"),
                executable: &fixture.exe,
                args: vec!["mosdns".into(), "start".into()],
                health_url: "http://127.0.0.1:9/api/v1/system/health".into(),
                health_timeout_secs: 5,
                kind: "binary",
                previous_version: "v0.1.0",
                target_version: "v0.2.0",
                asset_signature: "asset:abc",
                candidate: Some(b"NEW-PROGRAM".as_slice()),
                package: None,
                webui: Some(webui),
            },
            &|_, _| Ok(()),
        )
    }

    fn live_index(fixture: &Fixture) -> Option<String> {
        std::fs::read_to_string(fixture.root.join("ui/main/index.html")).ok()
    }

    #[test]
    fn staging_a_webui_checks_and_keeps_the_archive_and_changes_nothing_live() {
        let fixture = fixture("webui-stage");
        std::fs::create_dir_all(fixture.root.join("ui/main")).unwrap();
        std::fs::write(fixture.root.join("ui/main/index.html"), "OLD-UI").unwrap();
        let archive = good_webui("NEW-UI");
        let (path, tx) = stage_webui(&fixture, &archive).unwrap();
        let dir = path.parent().unwrap();
        assert_eq!(std::fs::read(&tx.webui_archive_path).unwrap(), archive);
        assert_eq!(tx.webui_sha256, sha256_hex(&archive));
        assert_eq!(Path::new(&tx.webui_new_dir), dir.join("webui/new"));
        assert_eq!(
            Path::new(&tx.webui_previous_dir),
            dir.join("webui/previous")
        );
        assert!(
            !dir.join("webui/check").exists(),
            "the check tree is removed"
        );
        assert_eq!(live_index(&fixture).as_deref(), Some("OLD-UI"));
        validate_paths(&tx).unwrap();
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_webui_archive_that_is_no_webui_package_is_refused_before_anything_is_staged() {
        let fixture = fixture("webui-bad");
        std::fs::create_dir_all(fixture.root.join("ui/main")).unwrap();
        for (archive, reason) in [
            (
                webui_tar(&[(
                    "ui-manifest.json",
                    br#"{"manifest_schema":1,"ui_version":"v"}"#,
                )]),
                "index.html",
            ),
            (webui_tar(&[("index.html", b"x")]), "ui-manifest.json"),
            (
                webui_tar(&[("index.html", b"x"), ("ui-manifest.json", b"{")]),
                "JSON",
            ),
            (
                webui_tar(&[
                    ("index.html", b"x"),
                    (
                        "ui-manifest.json",
                        br#"{"manifest_schema":2,"ui_version":"v"}"#,
                    ),
                ]),
                "manifest_schema",
            ),
            (webui_tar(&[("../x", b"x")]), "解压"),
            (b"not gzip".to_vec(), "解压"),
        ] {
            let error = stage_webui(&fixture, &archive).unwrap_err();
            assert!(error.contains(reason), "{reason}: {error}");
            assert!(list(&fixture.root).is_empty(), "{reason}");
        }
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn the_webui_is_installed_and_rolled_back_with_the_program_and_the_rollback_repeats() {
        let fixture = fixture("webui-rollback");
        std::fs::create_dir_all(fixture.root.join("ui/main")).unwrap();
        std::fs::write(fixture.root.join("ui/main/index.html"), "OLD-UI").unwrap();
        let (path, mut tx) = stage_webui(&fixture, &good_webui("NEW-UI")).unwrap();
        // A rollback before anything was installed leaves `ui/main` alone.
        let mut early = tx.clone();
        rollback(&path, &mut early, "未开始").unwrap();
        assert_eq!(live_index(&fixture).as_deref(), Some("OLD-UI"));
        tx.status = "installing".into();
        write(&path, &tx).unwrap();
        crate::update::webui::install(&tx).unwrap();
        durable_copy(Path::new(&tx.candidate_path), &fixture.exe, 0o755).unwrap();
        assert_eq!(live_index(&fixture).as_deref(), Some("NEW-UI"));
        assert_eq!(
            std::fs::read(fixture.root.join("ui/main/chunks/a.js")).unwrap(),
            b"js"
        );
        rollback(&path, &mut tx, "候选未通过").unwrap();
        assert_eq!(live_index(&fixture).as_deref(), Some("OLD-UI"));
        assert!(!fixture.root.join("ui/main/chunks").exists());
        assert_eq!(std::fs::read(&fixture.exe).unwrap(), b"OLD-PROGRAM");
        rollback(&path, &mut tx, "候选未通过").unwrap();
        assert_eq!(live_index(&fixture).as_deref(), Some("OLD-UI"));
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn an_install_interrupted_between_its_two_renames_is_recovered_at_start() {
        let fixture = fixture("webui-interrupted");
        std::fs::create_dir_all(fixture.root.join("ui/main")).unwrap();
        std::fs::write(fixture.root.join("ui/main/index.html"), "OLD-UI").unwrap();
        let (path, mut tx) = stage_webui(&fixture, &good_webui("NEW-UI")).unwrap();
        tx.status = "installing".into();
        write(&path, &tx).unwrap();
        // The old directory was moved aside; power was lost before the new one went in.
        std::fs::create_dir_all(Path::new(&tx.webui_previous_dir).parent().unwrap()).unwrap();
        std::fs::rename(fixture.root.join("ui/main"), &tx.webui_previous_dir).unwrap();
        assert!(live_index(&fixture).is_none());
        let recovered = recover_on_start(&fixture.root).unwrap();
        assert!(recovered.is_some());
        assert_eq!(live_index(&fixture).as_deref(), Some("OLD-UI"));
        assert_eq!(read(&path).unwrap().status, "rolled_back");
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn webui_presence_replaces_only_a_real_directory() {
        use crate::update::webui::{Presence, presence};
        let fixture = fixture("webui-presence");
        assert_eq!(presence(&fixture.root), Presence::Absent);
        std::fs::create_dir_all(fixture.root.join("ui")).unwrap();
        assert_eq!(presence(&fixture.root), Presence::Absent);
        std::fs::create_dir_all(fixture.root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(fixture.root.join("elsewhere"), fixture.root.join("ui/main"))
            .unwrap();
        assert_eq!(presence(&fixture.root), Presence::Unusable);
        std::fs::remove_file(fixture.root.join("ui/main")).unwrap();
        std::fs::write(fixture.root.join("ui/main"), "a file").unwrap();
        assert_eq!(presence(&fixture.root), Presence::Unusable);
        std::fs::remove_file(fixture.root.join("ui/main")).unwrap();
        std::fs::create_dir(fixture.root.join("ui/main")).unwrap();
        assert_eq!(presence(&fixture.root), Presence::Directory);
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn a_transaction_written_before_the_webui_fields_still_reads() {
        let fixture = fixture("webui-old-format");
        let (path, tx) = stage_for(&fixture, "binary", false, &|_, _| Ok(())).unwrap();
        let mut value = serde_json::to_value(&tx).unwrap();
        for key in [
            "webui_archive_path",
            "webui_sha256",
            "webui_new_dir",
            "webui_previous_dir",
        ] {
            value.as_object_mut().unwrap().remove(key);
        }
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let read_back = read(&path).unwrap();
        assert!(read_back.webui_archive_path.is_empty());
        validate_paths(&read_back).unwrap();
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn doctored_webui_paths_are_refused_even_inside_the_update_directory() {
        let fixture = fixture("webui-doctored");
        std::fs::create_dir_all(fixture.root.join("ui/main")).unwrap();
        std::fs::write(fixture.root.join("ui/main/index.html"), "OLD-UI").unwrap();
        let (path, tx) = stage_webui(&fixture, &good_webui("NEW-UI")).unwrap();
        validate_paths(&tx).unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        for (field, value) in [
            ("new", update_dir(&fixture.root)),
            ("new", dir.clone()),
            ("previous", dir.join("config-backup")),
            ("archive", dir.join("mosdns.previous")),
        ] {
            let mut doctored = tx.clone();
            let value = value.display().to_string();
            match field {
                "new" => doctored.webui_new_dir = value,
                "previous" => doctored.webui_previous_dir = value,
                _ => doctored.webui_archive_path = value,
            }
            assert!(validate_paths(&doctored).is_err(), "{field}");
            let mut copy = doctored.clone();
            assert!(
                rollback(&path, &mut copy, "x").is_err(),
                "{field}: nothing is touched"
            );
        }
        let mut no_txn = tx.clone();
        no_txn.previous_binary_path = update_dir(&fixture.root)
            .join("mosdns.previous")
            .display()
            .to_string();
        assert!(validate_paths(&no_txn).is_err());
        assert!(dir.join("mosdns.previous").exists() && dir.join("rtdns-webui.tar.gz").exists());
        std::fs::remove_dir_all(fixture.root).unwrap();
    }

    #[test]
    fn the_webui_is_never_moved_through_a_symlinked_ui_and_an_unreadable_backup_blocks_the_rollback()
     {
        let fixture = fixture("webui-ui-link");
        std::fs::create_dir_all(fixture.root.join("ui/main")).unwrap();
        std::fs::write(fixture.root.join("ui/main/index.html"), "OLD-UI").unwrap();
        let (path, mut tx) = stage_webui(&fixture, &good_webui("NEW-UI")).unwrap();
        // `ui` replaced by a link to another directory after staging: install refuses.
        let elsewhere = fixture.root.join("elsewhere");
        std::fs::rename(fixture.root.join("ui"), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, fixture.root.join("ui")).unwrap();
        let error = crate::update::webui::install(&tx).unwrap_err();
        assert!(error.contains("ui/"), "{error}");
        assert_eq!(
            std::fs::read_to_string(elsewhere.join("main/index.html")).unwrap(),
            "OLD-UI"
        );
        std::fs::remove_file(fixture.root.join("ui")).unwrap();
        std::fs::rename(&elsewhere, fixture.root.join("ui")).unwrap();
        // Installed for real, then `ui` becomes a link: the rollback refuses and keeps its state.
        tx.status = "testing".into();
        write(&path, &tx).unwrap();
        crate::update::webui::install(&tx).unwrap();
        std::fs::rename(fixture.root.join("ui"), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, fixture.root.join("ui")).unwrap();
        assert!(rollback(&path, &mut tx.clone(), "x").is_err());
        assert_eq!(read(&path).unwrap().status, "testing");
        std::fs::remove_file(fixture.root.join("ui")).unwrap();
        std::fs::rename(&elsewhere, fixture.root.join("ui")).unwrap();
        // A backup that cannot be looked at is not "no backup".
        let webui = Path::new(&tx.webui_previous_dir)
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::rename(&webui, fixture.root.join("webui-aside")).unwrap();
        std::fs::write(&webui, "not a directory").unwrap();
        assert!(crate::update::webui::restore(&tx).is_err());
        std::fs::remove_file(&webui).unwrap();
        std::fs::rename(fixture.root.join("webui-aside"), &webui).unwrap();
        rollback(&path, &mut tx, "候选未通过").unwrap();
        assert_eq!(live_index(&fixture).as_deref(), Some("OLD-UI"));
        std::fs::remove_dir_all(fixture.root).unwrap();
    }
}
