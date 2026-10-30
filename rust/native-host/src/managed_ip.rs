//! IP management uses the existing query handle, file lock and atomic writer.
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use crate::managed::{PersistFault, PersistGate, persist_rules_atomically};
use crate::policy::{IpListHandle, POLICY_BYTES_LIMIT, POLICY_RULE_LIMIT, ip_prefix};

pub(crate) enum ReplaceError {
    Limits,
    Failed(String),
}

pub(crate) struct ManagedIpSet {
    pub(crate) file: Option<PathBuf>,
    pub(crate) reason: Option<String>,
    prefixes: Rc<IpListHandle>,
    fault: Cell<PersistFault>,
    gate: RefCell<Option<PersistGate>>,
    update: Arc<tokio::sync::Mutex<()>>,
}

impl ManagedIpSet {
    pub(crate) fn new(inline: &[String], files: &[PathBuf], prefixes: Rc<IpListHandle>) -> Self {
        let reason = if !inline.is_empty() {
            Some("inline IPs are configured".into())
        } else if files.len() != 1 {
            Some(format!(
                "requires one .txt file; {} files configured",
                files.len()
            ))
        } else if !files[0]
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
        {
            Some("the file is not a .txt source".into())
        } else {
            None
        };
        // Write the actual source, preserving a declared symlink to it.
        let file = files.first().map(|file| actual_path(file));
        let update = crate::rule_sources::write_lock(file.clone().unwrap_or_default());
        Self {
            file,
            reason,
            prefixes,
            fault: Cell::new(PersistFault::None),
            gate: RefCell::new(None),
            update,
        }
    }

    pub(crate) fn show(&self) -> String {
        self.prefixes
            .current()
            .prefixes()
            .map(|(ip, bits)| format!("{ip}/{bits}\n"))
            .collect()
    }

    pub(crate) async fn replace(&self, values: Vec<String>) -> Result<usize, ReplaceError> {
        let _lock = self.update.lock().await;
        let file = self
            .file
            .clone()
            .ok_or_else(|| ReplaceError::Failed("no writable file".into()))?;
        let fault = self.fault.get();
        let gate = self.gate.borrow().clone();
        let list = crate::rule_sources::on_build_thread(move || {
            if values.len() > POLICY_RULE_LIMIT
                || values.iter().map(String::len).sum::<usize>() > POLICY_BYTES_LIMIT
            {
                return Err(ReplaceError::Limits);
            }
            let mut list = atomdns_matcher_core::IpPrefixList::new();
            for value in values {
                if let Ok((ip, bits)) = ip_prefix(&value, "post") {
                    list.append(ip, bits);
                }
            }
            list.rebuild();
            let rules = list
                .prefixes()
                .map(|(ip, bits)| format!("{ip}/{bits}"))
                .collect::<Vec<_>>();
            if let Some(gate) = gate {
                gate.wait();
            }
            persist_rules_atomically(&file, &rules, fault)
                .map_err(|error| ReplaceError::Failed(error.to_string()))?;
            Ok(list)
        })
        .await
        .map_err(ReplaceError::Failed)??;
        let count = list.len();
        self.prefixes.publish(Rc::new(list));
        Ok(count)
    }

    #[cfg(test)]
    pub(crate) fn inject_gate(&self, gate: PersistGate) {
        *self.gate.borrow_mut() = Some(gate);
    }
}

/// Resolve existing ancestors too, so aliases of a missing optional file collide.
pub(crate) fn actual_path(path: &Path) -> PathBuf {
    if let Some(canonical) = crate::config::canonicalized_path(path) {
        return canonical;
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while let Some(parent) = ancestor.parent() {
        if let Some(name) = ancestor.file_name() {
            suffix.push(name.to_os_string());
        }
        ancestor = parent;
        if let Some(mut canonical) = crate::config::canonicalized_path(ancestor) {
            for name in suffix.iter().rev() {
                canonical.push(name);
            }
            return crate::special_groups::lexical_path(&canonical);
        }
    }
    crate::config::normalized_artifact(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_ip_writes_preserve_the_file_and_live_list() {
        let root = std::env::temp_dir().join(format!("ip-write-fail-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("ips.txt");
        std::fs::write(&file, "192.0.2.7\n").unwrap();
        let prefixes = crate::policy::ip_list(&[], std::slice::from_ref(&file), "test").unwrap();
        let set = ManagedIpSet::new(&[], std::slice::from_ref(&file), prefixes);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for fault in [PersistFault::WriteTemp, PersistFault::Rename] {
                set.fault.set(fault);
                assert!(set.replace(vec!["198.51.100.1".into()]).await.is_err());
                assert_eq!(set.show(), "192.0.2.7/32\n");
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "192.0.2.7\n");
                assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn input_limits_are_distinct_from_write_failures_and_change_nothing() {
        let root = std::env::temp_dir().join(format!("ip-input-limits-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("ips.txt");
        std::fs::write(&file, "192.0.2.7\n").unwrap();
        let prefixes = crate::policy::ip_list(&[], std::slice::from_ref(&file), "test").unwrap();
        let set = ManagedIpSet::new(&[], std::slice::from_ref(&file), prefixes);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for values in [
                vec![String::new(); POLICY_RULE_LIMIT + 1],
                vec!["x".repeat(POLICY_BYTES_LIMIT + 1)],
            ] {
                assert!(matches!(
                    set.replace(values).await,
                    Err(ReplaceError::Limits)
                ));
                assert_eq!(set.show(), "192.0.2.7/32\n");
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "192.0.2.7\n");
                assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
