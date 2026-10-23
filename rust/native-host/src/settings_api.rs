//! HTTP routes of the typed settings layer: the WebUI's `/api/v1/overrides`,
//! `/api/v1/domain-generation`, `/api/v1/flags`, the effective-settings report
//! and the settings-backed `POST /api/v1/upstream/config`. Every change builds new
//! settings and goes through `RuntimeControl::apply_settings` (a whole
//! generation, or nothing). See `docs/rust/contracts/native-settings.md`.

use serde::Deserialize;

use crate::config::CompiledConfig;
use crate::config::flags::{Flag, FlagValue};
use crate::config::settings::{
    NO_V4, NO_V6, PluginSwitch, REMEMBER_DIRECT, REMEMBER_PROXY, UpstreamEntry,
};
use crate::rule_sources::Reply;
use crate::runtime_snapshot::{ApplyFailure, RuntimeControl};

const JSON: &str = "application/json; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

fn text(status: u16, message: &str) -> Reply {
    Reply {
        status,
        content_type: Some(TEXT),
        body: format!("{message}\n").into_bytes(),
        headers: Vec::new(),
    }
}

fn json(status: u16, value: &serde_json::Value) -> Reply {
    let mut body = serde_json::to_vec(value).unwrap_or_default();
    body.push(b'\n');
    Reply {
        status,
        content_type: Some(JSON),
        body,
        headers: Vec::new(),
    }
}

fn not_allowed() -> Reply {
    Reply {
        status: 405,
        content_type: Some(TEXT),
        body: b"method not allowed\n".to_vec(),
        headers: Vec::new(),
    }
}

/// An apply failure as the HTTP answer.
fn failure(error: ApplyFailure) -> Reply {
    match error {
        ApplyFailure::Invalid(reason) => text(400, &reason),
        ApplyFailure::Busy(reason) => text(409, &format!("settings are being applied: {reason}")),
        ApplyFailure::Conflict(path) => text(
            409,
            &format!("{} changed while applying; retry", path.display()),
        ),
        ApplyFailure::ResourceConflict(reason) => text(409, &reason),
        ApplyFailure::Failed(reason) => text(500, &reason),
        ApplyFailure::Shutdown => text(503, "host is stopping"),
        ApplyFailure::RecoveryRequired(reason) => {
            text(503, &format!("recovery required: {reason}"))
        }
        ApplyFailure::CommittedCleanup { generation, reason } => text(
            500,
            &format!("settings applied as generation {generation}, but cleanup failed: {reason}"),
        ),
    }
}

/// The UI shape of one upstream entry: `use_socks_proxy` is the effective
/// proxy switch (the reference field name).
fn entry_json(entry: &UpstreamEntry) -> serde_json::Value {
    let mut value = serde_json::to_value(entry).unwrap_or_default();
    if let Some(map) = value.as_object_mut() {
        // The page's field is the reference name, `use_socks_proxy`.
        map.remove("use_proxy");
        map.insert(
            "use_socks_proxy".to_owned(),
            serde_json::Value::Bool(entry.proxy_switch()),
        );
    }
    value
}

/// `GET /api/v1/upstream/config`: every `aliapi` tag with its effective list.
pub(crate) fn upstream_config(config: &CompiledConfig) -> serde_json::Value {
    serde_json::Value::Object(
        config
            .effective_upstreams
            .iter()
            .map(|(tag, entries)| {
                (
                    tag.clone(),
                    serde_json::Value::Array(entries.iter().map(entry_json).collect()),
                )
            })
            .collect(),
    )
}

/// The settings entries of `tag`, for `GET /runtime/{tag}`.
pub(crate) fn override_config(config: &CompiledConfig, tag: &str) -> Vec<serde_json::Value> {
    config
        .settings
        .upstreams
        .get(tag)
        .map(|entries| entries.iter().map(entry_json).collect())
        .unwrap_or_default()
}

/// `POST /api/v1/upstream/config {plugin_tag, upstreams}`.
pub(crate) async fn post_upstream(control: &RuntimeControl, body: &[u8]) -> Reply {
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        plugin_tag: String,
        #[serde(default)]
        upstreams: Vec<UpstreamEntry>,
    }
    let payload: Payload = match serde_json::from_slice(body) {
        Ok(payload) => payload,
        Err(error) => {
            return text(
                400,
                &format!("invalid upstream configuration JSON: {error}"),
            );
        }
    };
    let tag = payload.plugin_tag.trim().to_owned();
    if tag.is_empty() {
        return text(400, "plugin_tag is required");
    }
    let current = control.view();
    if !current.config.effective_upstreams.contains_key(&tag) {
        return text(404, "upstream tag not found");
    }
    for (index, entry) in payload.upstreams.iter().enumerate() {
        if let Err(reason) = entry.validate(index) {
            return text(400, &reason);
        }
    }
    if let Some(profile) = &current.config.standard_groups {
        if profile
            .groups
            .iter()
            .any(|group| format!("special_upstream_{}", group.slot) == tag)
        {
            return text(400, "专属分流组上游只能在专属分流组设置中维护");
        }
    }
    let mut settings = current.config.settings.clone();
    let mut entries = payload.upstreams;
    for entry in &mut entries {
        entry.use_proxy = Some(entry.proxy_switch());
        entry.use_socks_proxy = None;
    }
    // Only differences are stored: an empty list, or one that says what the
    // YAML says, removes the stored list.
    let canonical = |list: &[UpstreamEntry]| {
        list.iter()
            .map(UpstreamEntry::canonical)
            .collect::<Vec<_>>()
    };
    let same_as_yaml = current
        .config
        .yaml_defaults
        .upstreams
        .get(&tag)
        .is_some_and(|yaml| canonical(yaml) == canonical(&entries));
    if entries.is_empty() || same_as_yaml {
        settings.upstreams.remove(&tag);
    } else {
        settings.upstreams.insert(tag, entries);
    }
    drop(current);
    match control.apply_settings(settings).await {
        Ok(_) => json(
            200,
            &serde_json::json!({ "message": "Upstream configuration saved." }),
        ),
        Err(error) => failure(error),
    }
}

/// The overrides page exists when a `socks5_proxy` or `ecs_address` does.
pub(crate) fn overrides_available(config: &CompiledConfig) -> bool {
    config.reload.is_some()
        && config
            .settings_report
            .iter()
            .any(|item| matches!(item.kind, "socks5_proxy" | "ecs_address"))
}

/// The domain-generation switches exist when a `domain_output` does.
pub(crate) fn generation_available(config: &CompiledConfig) -> bool {
    config.reload.is_some()
        && (!config.local.domain_outputs.is_empty() || !config.local.learners.is_empty())
}

/// The flag routes exist when a file-backed configuration declares a flag.
pub(crate) fn flags_available(config: &CompiledConfig) -> bool {
    config.reload.is_some() && !config.flags.is_empty()
}

/// The tags of one plugin kind in the effective report, in order.
fn objects<'a>(config: &'a CompiledConfig, kind: &str) -> Vec<&'a crate::config::EffectiveItem> {
    config
        .settings_report
        .iter()
        .filter(|item| item.kind == kind)
        .collect()
}

/// `/api/v1/overrides`, `/api/v1/domain-generation`, `/api/v1/flags[/{key}]`,
/// `/api/v1/settings/effective`; `None` for any other path.
pub(crate) async fn handle(
    control: Option<&RuntimeControl>,
    config: &CompiledConfig,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Reply> {
    let reply = match path.trim_end_matches('/') {
        "/api/v1/settings/effective" => match method {
            "GET" => effective(config),
            _ => not_allowed(),
        },
        "/api/v1/flags" if !flags_available(config) => return None,
        "/api/v1/flags" => match method {
            "GET" => flags_get(config),
            _ => not_allowed(),
        },
        other if other.starts_with("/api/v1/flags/") => {
            if !flags_available(config) {
                return None;
            }
            let key = &other["/api/v1/flags/".len()..];
            match method {
                "POST" | "DELETE" => match control {
                    Some(control) => flag_change(control, method, key, body).await,
                    None => text(503, "runtime control is unavailable"),
                },
                _ => not_allowed(),
            }
        }
        "/api/v1/overrides" if !overrides_available(config) => return None,
        "/api/v1/domain-generation" if !generation_available(config) => return None,
        "/api/v1/overrides" => match method {
            "GET" => overrides_get(config),
            "POST" => match control {
                Some(control) => overrides_post(control, body).await,
                None => text(503, "runtime control is unavailable"),
            },
            _ => not_allowed(),
        },
        "/api/v1/domain-generation" => match method {
            "GET" => generation_get(config),
            "POST" => match control {
                Some(control) => generation_post(control, body).await,
                None => text(503, "runtime control is unavailable"),
            },
            _ => not_allowed(),
        },
        _ => return None,
    };
    Some(reply)
}

fn flag_json(flag: &Flag) -> serde_json::Value {
    let mut value = serde_json::json!({
        "key": flag.key,
        "label": flag.label,
        "description": flag.description,
        "section": flag.section,
        "kind": flag.kind(),
        "default": flag.default,
        "value": flag.value,
        "modified": flag.value != flag.default,
        "source": flag.source,
    });
    if let Some(options) = &flag.options {
        value["options"] = options
            .iter()
            .map(|option| serde_json::json!({"value": option.value, "label": option.label}))
            .collect();
    }
    value
}

fn flags_get(config: &CompiledConfig) -> Reply {
    json(
        200,
        &serde_json::json!({
            "settings_file": crate::config::settings::SETTINGS_FILE,
            "migrated": config.settings.flags.is_some(),
            "flags": config.flags.iter().map(flag_json).collect::<Vec<_>>(),
        }),
    )
}

/// `POST /api/v1/flags/{key}` (`{"value": …}`) and `DELETE` (back to the
/// default). Only the differences from the defaults are stored; a flag set to
/// its default leaves the `flags` section (which stays, possibly empty).
async fn flag_change(control: &RuntimeControl, method: &str, key: &str, body: &[u8]) -> Reply {
    let current = control.view();
    let Some(flag) = current.config.flags.iter().find(|flag| flag.key == key) else {
        return text(404, &format!("no feature_flag `{key}`"));
    };
    let value = if method == "DELETE" {
        flag.default.clone()
    } else {
        let Ok(serde_json::Value::Object(object)) = serde_json::from_slice(body) else {
            return text(400, "expected a JSON object {\"value\": ...}");
        };
        if object.len() != 1 {
            return text(400, "expected exactly one field: value");
        }
        let value = match object.get("value") {
            Some(serde_json::Value::Bool(value)) => FlagValue::Bool(*value),
            Some(serde_json::Value::String(value)) => FlagValue::Choice(value.clone()),
            _ => return text(400, "value must be a boolean or an option string"),
        };
        if !flag.accepts(&value) {
            return text(
                400,
                &format!(
                    "`{value}` is not a valid value of the {} flag `{key}`",
                    flag.kind()
                ),
            );
        }
        value
    };
    if crate::config::flags::update_trial() {
        return text(503, "配置更新试运行中，暂不能修改开关");
    }
    if value == flag.value {
        return json(200, &flag_json(flag));
    }
    let mut settings = current.config.settings.clone();
    // Before the first migration the values may come from the legacy switch
    // files; the first change stores all of them.
    let mut stored = settings
        .flags
        .take()
        .unwrap_or_else(|| crate::config::flags::differences(&current.config.flags));
    if value == flag.default {
        stored.remove(key);
    } else {
        stored.insert(key.to_owned(), value);
    }
    settings.flags = Some(stored);
    drop(current);
    if let Err(error) = control.apply_settings(settings).await {
        return failure(error);
    }
    let view = control.view();
    match view.config.flags.iter().find(|flag| flag.key == key) {
        Some(flag) => json(200, &flag_json(flag)),
        None => text(500, "the flag is gone after the change"),
    }
}

fn effective(config: &CompiledConfig) -> Reply {
    json(
        200,
        &serde_json::json!({
            "settings_file": crate::config::settings::SETTINGS_FILE,
            "items": config.settings_report,
        }),
    )
}

/// The one object of a kind, or why there is not exactly one.
fn unique<'a>(
    config: &'a CompiledConfig,
    kind: &str,
) -> Result<Option<&'a crate::config::EffectiveItem>, String> {
    let all = objects(config, kind);
    match all.len() {
        0 => Ok(None),
        1 => Ok(Some(all[0])),
        _ => Err(format!(
            "ambiguous: several {kind} objects ({}); the WebUI field maps to exactly one",
            all.iter()
                .map(|item| item.object.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The address of the configured `socks5_proxy` object (empty when there is none, or it is
/// ambiguous): the proxy the updater's downloads go through.
pub(crate) fn proxy_address(config: &CompiledConfig) -> String {
    unique(config, "socks5_proxy")
        .ok()
        .flatten()
        .map(|item| item.value.trim().to_owned())
        .unwrap_or_default()
}

fn overrides_get(config: &CompiledConfig) -> Reply {
    let value = |kind: &str| -> Result<String, String> {
        Ok(unique(config, kind)?
            .map(|item| item.value.clone())
            .unwrap_or_default())
    };
    match (value("socks5_proxy"), value("ecs_address")) {
        (Ok(socks5), Ok(ecs)) => json(
            200,
            &serde_json::json!({ "socks5": socks5, "ecs": ecs, "replacements": [] }),
        ),
        (Err(reason), _) | (_, Err(reason)) => text(409, &reason),
    }
}

async fn overrides_post(control: &RuntimeControl, body: &[u8]) -> Reply {
    #[derive(Deserialize)]
    struct Replacement {
        #[serde(default)]
        original: String,
    }
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        socks5: String,
        #[serde(default)]
        ecs: String,
        #[serde(default)]
        replacements: Option<Vec<Replacement>>,
    }
    let payload: Payload = match serde_json::from_slice(body) {
        Ok(payload) => payload,
        Err(error) => return text(400, &format!("Invalid request body: {error}")),
    };
    if payload
        .replacements
        .as_ref()
        .is_some_and(|rules| rules.iter().any(|rule| !rule.original.trim().is_empty()))
    {
        return text(
            400,
            "replacements are not supported; use socks5_proxy / ecs_address objects",
        );
    }
    let current = control.view();
    let mut settings = current.config.settings.clone();
    for (kind, wanted, section) in [
        ("socks5_proxy", payload.socks5.trim(), "proxies"),
        ("ecs_address", payload.ecs.trim(), "ecs"),
    ] {
        let object = match unique(&current.config, kind) {
            Ok(object) => object,
            Err(reason) => return text(409, &reason),
        };
        let Some(object) = object else {
            if wanted.is_empty() {
                continue;
            }
            return text(400, &format!("no {kind} object is configured"));
        };
        let map = if section == "proxies" {
            &mut settings.proxies
        } else {
            &mut settings.ecs
        };
        let yaml = if section == "proxies" {
            &current.config.yaml_defaults.proxies
        } else {
            &current.config.yaml_defaults.ecs
        };
        if wanted.is_empty()
            || yaml
                .get(&object.object)
                .is_some_and(|value| value == wanted)
        {
            // Back to the YAML value.
            map.remove(&object.object);
        } else {
            map.insert(
                object.object.clone(),
                crate::config::settings::Addr {
                    addr: wanted.to_owned(),
                },
            );
        }
    }
    let unchanged = settings == current.config.settings;
    drop(current);
    if !unchanged {
        if let Err(error) = control.apply_settings(settings).await {
            return failure(error);
        }
    }
    json(
        200,
        &serde_json::json!({ "message": "Global overrides saved and applied." }),
    )
}

/// The five WebUI switches from the effective report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Generation {
    enabled: bool,
    remember_direct: bool,
    remember_proxy: bool,
    no_v4: bool,
    no_v6: bool,
}

impl Generation {
    fn of(config: &CompiledConfig) -> Self {
        let settings = &config.settings;
        // A plugin's own setting, else what the YAML says, else on.
        let plugin = |tag: &str| {
            settings
                .domain_generation
                .plugins
                .get(tag)
                .map(|switch| switch.enabled)
                .or_else(|| config.yaml_defaults.generation.get(tag).copied())
                .unwrap_or(true)
        };
        Self {
            enabled: settings.domain_generation.enabled.unwrap_or(true),
            remember_direct: plugin(REMEMBER_DIRECT),
            remember_proxy: plugin(REMEMBER_PROXY),
            no_v4: plugin(NO_V4),
            no_v6: plugin(NO_V6),
        }
    }

    fn json(self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "remember_direct": self.remember_direct,
            "remember_proxy": self.remember_proxy,
            "no_v4": self.no_v4,
            "no_v6": self.no_v6,
        })
    }
}

fn generation_get(config: &CompiledConfig) -> Reply {
    json(200, &Generation::of(config).json())
}

async fn generation_post(control: &RuntimeControl, body: &[u8]) -> Reply {
    #[derive(Deserialize)]
    struct Patch {
        enabled: Option<bool>,
        remember_direct: Option<bool>,
        remember_proxy: Option<bool>,
        no_v4: Option<bool>,
        no_v6: Option<bool>,
    }
    let patch: Patch = match serde_json::from_slice(body) {
        Ok(patch) => patch,
        Err(error) => return text(400, &error.to_string()),
    };
    let current = control.view();
    let before = Generation::of(&current.config);
    let mut after = before;
    if let Some(value) = patch.enabled {
        after.enabled = value;
    }
    if let Some(value) = patch.remember_direct {
        after.remember_direct = value;
    }
    if let Some(value) = patch.remember_proxy {
        after.remember_proxy = value;
    }
    if let Some(value) = patch.no_v4 {
        after.no_v4 = value;
    }
    if let Some(value) = patch.no_v6 {
        after.no_v6 = value;
    }
    let present: Vec<String> = current
        .config
        .local
        .domain_outputs
        .iter()
        .map(|output| output.tag.clone())
        .chain(
            current
                .config
                .local
                .learners
                .iter()
                .flat_map(|learner| learner.runtime.category_tags().map(str::to_owned)),
        )
        .collect();
    // The reference flushes a list whose switch went from on to off; the master
    // switch going off flushes every list.
    let mut flush: Vec<String> = Vec::new();
    if before.enabled && !after.enabled {
        flush.extend(present.iter().cloned());
    }
    for (tag, was, now) in [
        (
            REMEMBER_DIRECT,
            before.remember_direct,
            after.remember_direct,
        ),
        (REMEMBER_PROXY, before.remember_proxy, after.remember_proxy),
        (NO_V4, before.no_v4, after.no_v4),
        (NO_V6, before.no_v6, after.no_v6),
    ] {
        if was && !now && present.iter().any(|name| name == tag) {
            flush.push(tag.to_owned());
        }
    }
    flush.sort();
    flush.dedup();
    let mut settings = current.config.settings.clone();
    settings.domain_generation.enabled = (!after.enabled).then_some(false);
    for (tag, value) in [
        (REMEMBER_DIRECT, after.remember_direct),
        (REMEMBER_PROXY, after.remember_proxy),
        (NO_V4, after.no_v4),
        (NO_V6, after.no_v6),
    ] {
        let yaml = current.config.yaml_defaults.generation.get(tag).copied();
        if yaml.unwrap_or(true) == value {
            settings.domain_generation.plugins.remove(tag);
        } else {
            settings
                .domain_generation
                .plugins
                .insert(tag.to_owned(), PluginSwitch { enabled: value });
        }
    }
    let unchanged = settings == current.config.settings;
    drop(current);
    if !unchanged {
        if let Err(error) = control.apply_settings(settings).await {
            return failure(error);
        }
    }
    let mut failed = Vec::new();
    for tag in &flush {
        let path = format!("/plugins/{tag}/flush");
        match crate::api::local_plugin_request(control, "GET", &path, b"").await {
            Some((200..=299, _)) => {}
            Some((status, body)) => failed.push(format!(
                "{tag}: {status} {}",
                String::from_utf8_lossy(&body).trim()
            )),
            None => failed.push(format!("{tag}: no such plugin route")),
        }
    }
    if !failed.is_empty() {
        return text(
            500,
            &format!(
                "settings were applied, but flushing failed: {}",
                failed.join("; ")
            ),
        );
    }
    json(
        200,
        &serde_json::json!({ "settings": after.json(), "flush_targets": flush }),
    )
}
