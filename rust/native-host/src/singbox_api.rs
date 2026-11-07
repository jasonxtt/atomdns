//! Proxy-backend subscription and node management for the AtomDNS WebUI.
//!
//! The WebUI owns its subscription list in `webinfo/settings.json` and compiles
//! it into the backend's native config: sing-box `remote` providers plus a
//! provider-aware selector, or mihomo `proxy-providers` plus a `select` group.
//! Both backends serve the same `/atomdns/config` management API; mihomo says
//! so with `"backend": "mihomo"`. Node selection, latency tests and provider
//! updates use the live Clash API, which both backends implement.

use serde::{Deserialize, Serialize};

use crate::config::CompiledConfig;
use crate::config::settings::{Settings, SingboxManagement, SingboxNode, SingboxSubscription};
use crate::rule_sources::Reply;
use crate::runtime_snapshot::{ApplyFailure, RuntimeControl};

const MAX_BODY: usize = 16 * 1024 * 1024;

/// The proxy program behind the Clash API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    SingBox,
    Mihomo,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Mihomo => "mihomo",
        }
    }

    fn node_from_native(self, value: &serde_json::Value) -> Option<SingboxNode> {
        match self {
            Self::SingBox => SingboxNode::from_outbound(value),
            Self::Mihomo => SingboxNode::from_mihomo_proxy(value),
        }
    }
}

/// The backend config as read from `/atomdns/config`.
struct Managed {
    revision: String,
    config: serde_json::Value,
    backend: Backend,
}

#[derive(Clone)]
struct Client {
    base: String,
    secret: String,
}

#[derive(Deserialize)]
struct ProviderInput {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default = "default_interval")]
    update_interval_hours: u32,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default = "default_proxy")]
    use_proxy: bool,
}

const fn default_interval() -> u32 {
    24
}
const fn default_enabled() -> bool {
    true
}
const fn default_proxy() -> bool {
    true
}

#[derive(Deserialize)]
struct NodeInput {
    #[serde(default)]
    link: String,
    #[serde(default)]
    tag: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

#[derive(Serialize)]
struct SubscriptionView<'a> {
    name: &'a str,
    url: &'a str,
    update_interval_hours: u32,
    enabled: bool,
    use_proxy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_count: Option<usize>,
}

fn error(status: u16, message: impl AsRef<str>) -> Reply {
    let value = serde_json::json!({"error": message.as_ref()});
    let mut body = serde_json::to_vec(&value).unwrap_or_default();
    body.push(b'\n');
    Reply {
        status,
        content_type: Some("application/json; charset=utf-8"),
        body,
        headers: Vec::new(),
    }
}

fn success(value: serde_json::Value) -> Reply {
    let mut body = serde_json::to_vec(&value).unwrap_or_default();
    body.push(b'\n');
    Reply {
        status: 200,
        content_type: Some("application/json; charset=utf-8"),
        body,
        headers: Vec::new(),
    }
}

fn apply_error(failure: ApplyFailure) -> Reply {
    match failure {
        ApplyFailure::Invalid(reason) => error(400, reason),
        ApplyFailure::Busy(reason) => error(409, reason),
        ApplyFailure::Conflict(path) => error(409, format!("{} changed; retry", path.display())),
        ApplyFailure::ResourceConflict(reason) => error(409, reason),
        ApplyFailure::Failed(reason) => error(500, reason),
        ApplyFailure::Shutdown => error(503, "host is stopping"),
        ApplyFailure::RecoveryRequired(reason) => error(503, reason),
        ApplyFailure::CommittedCleanup { generation, reason } => error(
            500,
            format!("applied generation {generation}, cleanup failed: {reason}"),
        ),
    }
}

impl Client {
    fn new(config: &CompiledConfig) -> Result<Self, Reply> {
        let base =
            crate::config::settings::validate_singbox_addr(&config.settings.singbox_api.addr)
                .map_err(|reason| error(400, reason))?;
        if base.is_empty() {
            return Err(error(
                400,
                "请先在 上游设置 › 代理与 ECS 配置代理后端 API 地址与密钥",
            ));
        }
        Ok(Self {
            base,
            secret: config.settings.singbox_api.secret.clone(),
        })
    }

    async fn request(
        &self,
        method: &'static str,
        path: &str,
        body: Option<Vec<u8>>,
        seconds: u64,
    ) -> Result<(u16, Vec<u8>), String> {
        let url = format!("{}{path}", self.base);
        let cancel = atomdns_upstream_core::TransportCancellation::new();
        crate::rule_download::call(
            crate::rule_download::Call {
                method,
                url: &url,
                body: body.as_deref(),
                bearer_token: (!self.secret.is_empty()).then_some(self.secret.as_str()),
                socks5: "",
                timeout: std::time::Duration::from_secs(seconds),
                max_bytes: MAX_BODY,
                roots: None,
                follow: false,
                error_body: true,
            },
            &cancel,
        )
        .await
    }

    async fn json(&self, path: &str) -> Result<serde_json::Value, String> {
        let (status, bytes) = self.request("GET", path, None, 10).await?;
        if status != 200 {
            return Err(http_error("代理后端 API", status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|_| "proxy backend returned invalid JSON".to_owned())
    }

    async fn load_config(&self) -> Result<Managed, String> {
        let value = self.json("/atomdns/config").await?;
        let revision = value
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let config = value.get("config").cloned().unwrap_or_default();
        if revision.is_empty() || !config.is_object() {
            return Err(
                "proxy backend managed configuration is unavailable; use one writable config file"
                    .to_owned(),
            );
        }
        let backend = match value.get("backend").and_then(serde_json::Value::as_str) {
            Some("mihomo") => Backend::Mihomo,
            _ => Backend::SingBox,
        };
        Ok(Managed {
            revision,
            config,
            backend,
        })
    }

    /// `/proxies` with the nodes of every proxy provider merged in. sing-box
    /// lists provider nodes in `/proxies`; mihomo only under the providers.
    async fn proxy_details(&self) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        let mut map = self
            .json("/proxies")
            .await?
            .get("proxies")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Ok(value) = self.json("/providers/proxies").await
            && let Some(providers) = value
                .get("providers")
                .and_then(serde_json::Value::as_object)
        {
            for proxy in providers
                .values()
                .filter_map(|provider| provider.get("proxies"))
                .filter_map(serde_json::Value::as_array)
                .flatten()
            {
                if let Some(name) = proxy.get("name").and_then(serde_json::Value::as_str) {
                    map.entry(name.to_owned()).or_insert_with(|| proxy.clone());
                }
            }
        }
        Ok(map)
    }

    async fn put_config(&self, revision: &str, config: &serde_json::Value) -> Result<(), String> {
        let body = serde_json::to_vec(&serde_json::json!({"revision": revision, "config": config}))
            .map_err(|error| error.to_string())?;
        let (status, bytes) = self
            .request("PUT", "/atomdns/config", Some(body), 120)
            .await?;
        if status != 200 {
            return Err(http_error("代理后端配置", status, &bytes));
        }
        let accepted: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| "proxy backend returned an invalid save response".to_owned())?;
        let next_revision = accepted
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if next_revision.is_empty() {
            return Err("proxy backend did not return a configuration revision".to_owned());
        }
        for _ in 0..120 {
            let (status, bytes) = match self.request("GET", "/atomdns/config/status", None, 5).await
            {
                Ok(response) => response,
                Err(_) => {
                    // The old API listener closes before the replacement binds.
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            if status == 200 {
                let state: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|_| "proxy backend returned an invalid reload status".to_owned())?;
                if state.get("revision").and_then(serde_json::Value::as_str) == Some(next_revision)
                    && state.get("reloading").and_then(serde_json::Value::as_bool) == Some(false)
                {
                    if let Some(reason) = state.get("error").and_then(serde_json::Value::as_str) {
                        return Err(format!("代理后端配置应用失败，已恢复旧配置: {reason}"));
                    }
                    return Ok(());
                }
            } else if status != 503 {
                return Err(http_error("代理后端重载状态", status, &bytes));
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err("等待代理后端应用配置超时，请检查代理后端状态后重试".to_owned())
    }

    async fn parse_node(&self, input: &NodeInput, backend: Backend) -> Result<SingboxNode, String> {
        let body = serde_json::to_vec(
            &serde_json::json!({"link": input.link.trim(), "tag": input.tag.trim()}),
        )
        .map_err(|error| error.to_string())?;
        let (status, bytes) = self
            .request("POST", "/atomdns/config/parse-node", Some(body), 10)
            .await?;
        if status != 200 {
            return Err(http_error("节点链接解析", status, &bytes));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| "proxy backend returned an invalid node".to_owned())?;
        backend
            .node_from_native(&value)
            .ok_or_else(|| "proxy backend returned an incomplete node".to_owned())
    }
}

fn http_error(what: &str, status: u16, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
        && let Some(message) = value.get("message").and_then(serde_json::Value::as_str)
    {
        return format!("{what}: {message}");
    }
    format!(
        "{what}: HTTP {status}{}",
        if text.is_empty() {
            String::new()
        } else {
            format!(": {}", &text[..text.len().min(800)])
        }
    )
}

fn escape(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn decode(value: &str) -> String {
    url::form_urlencoded::parse(value.as_bytes())
        .next()
        .map_or_else(|| value.to_owned(), |(key, _)| key.into_owned())
}

fn subscriptions_json(management: &SingboxManagement) -> serde_json::Value {
    serde_json::to_value(management.subscriptions.as_slice()).unwrap_or_default()
}

fn is_group_or_builtin(item: &serde_json::Value) -> bool {
    matches!(
        item.get("type").and_then(serde_json::Value::as_str),
        Some("selector" | "urltest" | "loadbalance" | "direct" | "block")
    )
}

fn array<'a>(
    config: &'a serde_json::Value,
    key: &str,
) -> impl Iterator<Item = &'a serde_json::Value> {
    config
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
}

fn current_manual_nodes(
    config: &serde_json::Value,
    management: &SingboxManagement,
    backend: Backend,
) -> Vec<SingboxNode> {
    if management.nodes_managed {
        return management.nodes.clone();
    }
    match backend {
        Backend::SingBox => array(config, "outbounds")
            .filter(|item| !is_group_or_builtin(item))
            .filter_map(SingboxNode::from_outbound)
            .collect(),
        // mihomo keeps groups and providers apart from `proxies`.
        Backend::Mihomo => array(config, "proxies")
            .filter_map(SingboxNode::from_mihomo_proxy)
            .collect(),
    }
}

/// Every name a new manual node would collide with.
fn native_names(config: &serde_json::Value, backend: Backend) -> Vec<String> {
    let (keys, name_key): (&[&str], &str) = match backend {
        Backend::SingBox => (&["outbounds"], "tag"),
        Backend::Mihomo => (&["proxies", "proxy-groups"], "name"),
    };
    let mut names: Vec<String> = keys
        .iter()
        .flat_map(|key| array(config, key))
        .filter_map(|item| item.get(name_key).and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect();
    if backend == Backend::Mihomo {
        names.extend(
            [
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
                "GLOBAL",
            ]
            .map(str::to_owned),
        );
    }
    names
}

fn selector_tag(
    config: &serde_json::Value,
    management: &SingboxManagement,
    backend: Backend,
) -> String {
    if !management.group.is_empty() {
        return management.group.clone();
    }
    let (key, name_key, kind) = match backend {
        Backend::SingBox => ("outbounds", "tag", "selector"),
        Backend::Mihomo => ("proxy-groups", "name", "select"),
    };
    let selectors = || {
        array(config, key)
            .filter(move |item| item.get("type").and_then(serde_json::Value::as_str) == Some(kind))
    };
    selectors()
        .find(|item| item.get(name_key).and_then(serde_json::Value::as_str) == Some("proxy"))
        .or_else(|| selectors().next())
        .and_then(|item| item.get(name_key).and_then(serde_json::Value::as_str))
        .unwrap_or("proxy")
        .to_owned()
}

/// Applies the WebUI-owned pieces while preserving the rest of the backend
/// config.
fn managed_config(
    current: &serde_json::Value,
    management: &SingboxManagement,
    backend: Backend,
) -> Result<serde_json::Value, String> {
    match backend {
        Backend::SingBox => singbox_managed_config(current, management),
        Backend::Mihomo => mihomo_managed_config(current, management),
    }
}

fn singbox_managed_config(
    current: &serde_json::Value,
    management: &SingboxManagement,
) -> Result<serde_json::Value, String> {
    let mut result = current.clone();
    let object = result
        .as_object_mut()
        .ok_or("invalid sing-box configuration")?;
    let group = selector_tag(current, management, Backend::SingBox);
    let mut outbounds = object
        .get("outbounds")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if management.nodes_managed {
        // AtomDNS owns every manual node: the stored list replaces all generic
        // outbounds. Groups, direct/block and provider nodes stay.
        outbounds.retain(|item| {
            let tag = item
                .get("tag")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            tag.contains('/') || is_group_or_builtin(item)
        });
        outbounds.extend(
            management
                .nodes
                .iter()
                .filter(|node| node.enabled)
                .map(SingboxNode::outbound),
        );
    }
    let mut manual_tags: Vec<String> = outbounds
        .iter()
        .filter(|item| !is_group_or_builtin(item))
        .filter_map(|item| {
            item.get("tag")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .filter(|tag| !tag.contains('/'))
        .collect();
    let provider_tags: Vec<String> = management
        .subscriptions
        .iter()
        .filter(|subscription| subscription.enabled)
        .map(|subscription| subscription.name.clone())
        .collect();
    if manual_tags.is_empty() && provider_tags.is_empty() {
        // sing-box rejects an empty selector: with no node left, fall back to
        // direct so the configuration stays valid (fresh install, last delete).
        if !outbounds
            .iter()
            .any(|item| item.get("tag").and_then(serde_json::Value::as_str) == Some("direct"))
        {
            outbounds.push(serde_json::json!({"type": "direct", "tag": "direct"}));
        }
        manual_tags.push("direct".to_owned());
    }
    if management.providers_managed {
        let providers = management
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled)
            .map(|subscription| {
                serde_json::json!({
                    "type": "remote",
                    "tag": subscription.name,
                    "url": subscription.url,
                    "update_interval": format!("{}h", subscription.update_interval_hours),
                    "health_check": {"enabled": false},
                    "http_client": if subscription.use_proxy {
                        serde_json::json!({"detour": group})
                    } else {
                        serde_json::json!({})
                    },
                })
            })
            .collect::<Vec<_>>();
        object.insert("providers".to_owned(), serde_json::Value::Array(providers));
    }
    if management.providers_managed || management.nodes_managed {
        let mut found = false;
        for item in &mut outbounds {
            if item.get("tag").and_then(serde_json::Value::as_str) == Some(group.as_str()) {
                found = true;
                let selector = item.as_object_mut().ok_or("invalid selector")?;
                selector.insert(
                    "type".to_owned(),
                    serde_json::Value::String("selector".to_owned()),
                );
                selector.insert(
                    "outbounds".to_owned(),
                    serde_json::to_value(&manual_tags).unwrap_or_default(),
                );
                if management.providers_managed {
                    selector.insert(
                        "providers".to_owned(),
                        serde_json::to_value(&provider_tags).unwrap_or_default(),
                    );
                    selector.insert(
                        "use_all_providers".to_owned(),
                        serde_json::Value::Bool(false),
                    );
                }
                if let Some(default) = selector.get("default").and_then(serde_json::Value::as_str)
                    && !manual_tags.iter().any(|tag| tag == default)
                {
                    if let Some(first) = manual_tags.first() {
                        selector.insert(
                            "default".to_owned(),
                            serde_json::Value::String(first.clone()),
                        );
                    } else {
                        selector.remove("default");
                    }
                }
            }
        }
        if !found {
            outbounds.push(serde_json::json!({
                "type": "selector", "tag": group,
                "outbounds": manual_tags,
                "providers": provider_tags,
                "use_all_providers": false,
                "interrupt_exist_connections": false,
            }));
        }
        object.insert("outbounds".to_owned(), serde_json::Value::Array(outbounds));
    }
    Ok(result)
}

/// mihomo keeps manual nodes in `proxies`, subscriptions in `proxy-providers`
/// and the selector in `proxy-groups`, so AtomDNS owns each whole section it
/// manages. Provider nodes get a `<subscription>/` prefix like sing-box
/// provider tags, which keeps names unique and shows their source.
fn mihomo_managed_config(
    current: &serde_json::Value,
    management: &SingboxManagement,
) -> Result<serde_json::Value, String> {
    let mut result = current.clone();
    let object = result
        .as_object_mut()
        .ok_or("invalid mihomo configuration")?;
    let group = selector_tag(current, management, Backend::Mihomo);
    if management.nodes_managed {
        let proxies = management
            .nodes
            .iter()
            .filter(|node| node.enabled)
            .map(SingboxNode::mihomo_proxy)
            .collect();
        object.insert("proxies".to_owned(), serde_json::Value::Array(proxies));
    }
    let manual: Vec<String> = array(&result, "proxies")
        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect();
    let object = result
        .as_object_mut()
        .ok_or("invalid mihomo configuration")?;
    let provider_names: Vec<String> = management
        .subscriptions
        .iter()
        .filter(|subscription| subscription.enabled)
        .map(|subscription| subscription.name.clone())
        .collect();
    if management.providers_managed {
        let providers = management
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled)
            .map(|subscription| {
                // With no manual node the group holds only provider nodes,
                // which cannot carry their own first download.
                let detour = if subscription.use_proxy && !manual.is_empty() {
                    group.as_str()
                } else {
                    "DIRECT"
                };
                (
                    subscription.name.clone(),
                    serde_json::json!({
                        "type": "http",
                        "url": subscription.url,
                        "interval": u64::from(subscription.update_interval_hours) * 3600,
                        "proxy": detour,
                        "override": {"additional-prefix": format!("{}/", subscription.name)},
                        "health-check": {"enable": false},
                    }),
                )
            })
            .collect();
        object.insert(
            "proxy-providers".to_owned(),
            serde_json::Value::Object(providers),
        );
    }
    if management.providers_managed || management.nodes_managed {
        let mut members = manual.clone();
        if members.is_empty() && provider_names.is_empty() {
            // A select group needs a member: fall back to the built-in DIRECT
            // so the config stays valid (fresh install, last delete).
            members.push("DIRECT".to_owned());
        }
        let groups = object
            .entry("proxy-groups")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("invalid mihomo proxy-groups")?;
        let selector = match groups.iter_mut().find(|item| {
            item.get("name").and_then(serde_json::Value::as_str) == Some(group.as_str())
        }) {
            Some(selector) => selector,
            None => {
                groups.push(serde_json::json!({"name": group}));
                groups.last_mut().ok_or("invalid mihomo proxy-groups")?
            }
        };
        let selector = selector.as_object_mut().ok_or("invalid mihomo group")?;
        selector.insert("type".to_owned(), serde_json::json!("select"));
        if members.is_empty() {
            selector.remove("proxies");
        } else {
            selector.insert("proxies".to_owned(), serde_json::json!(members));
        }
        if management.providers_managed {
            if provider_names.is_empty() {
                selector.remove("use");
            } else {
                selector.insert("use".to_owned(), serde_json::json!(provider_names));
            }
        }
    }
    Ok(result)
}

/// The settings and backend configuration are a two-resource transaction:
/// validation and a config write precede a reload; failures restore the prior
/// settings or prior backend config rather than silently retaining a split state.
async fn save_managed(
    control: &RuntimeControl,
    config: &CompiledConfig,
    settings: Settings,
) -> Result<(), Reply> {
    let client = Client::new(config)?;
    let Managed {
        revision,
        config: current,
        backend,
    } = client
        .load_config()
        .await
        .map_err(|reason| error(502, reason))?;
    let name = backend.name();
    let next = managed_config(&current, &settings.singbox, backend)
        .map_err(|reason| error(400, reason))?;
    if next == current {
        return control
            .apply_settings(settings)
            .await
            .map(|_| ())
            .map_err(apply_error);
    }
    // Validate and persist the backend first. Only then persist AtomDNS
    // settings, so a rejected provider/node cannot leave the WebUI showing a
    // subscription that never loaded. If the second step fails, restore the
    // backend too.
    if let Err(reason) = client.put_config(&revision, &next).await {
        return Err(error(502, reason));
    }
    if let Err(failure) = control.apply_settings(settings).await {
        let latest = client.load_config().await.map_err(|reason| {
            error(
                502,
                format!("AtomDNS 设置未应用: {failure}; {name} revision read failed: {reason}"),
            )
        })?;
        let restored = client.put_config(&latest.revision, &current).await;
        return Err(error(
            502,
            match restored {
                Ok(()) => format!("AtomDNS 设置未应用，已恢复 {name} 配置: {failure}"),
                Err(reason) => {
                    format!("AtomDNS 设置未应用: {failure}; {name} 配置恢复也失败: {reason}")
                }
            },
        ));
    }
    Ok(())
}

fn decode_input<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, Reply> {
    serde_json::from_slice(body)
        .map_err(|decode_error| error(400, format!("Invalid request body: {decode_error}")))
}

fn parse_subscription(input: ProviderInput) -> Result<SingboxSubscription, Reply> {
    let subscription = SingboxSubscription {
        name: input.name.trim().to_owned(),
        url: input.url.trim().to_owned(),
        update_interval_hours: input.update_interval_hours,
        enabled: input.enabled,
        use_proxy: input.use_proxy,
    };
    let mut management = SingboxManagement {
        subscriptions: vec![subscription.clone()],
        ..SingboxManagement::default()
    };
    crate::config::settings::validate_singbox_management(&management)
        .map_err(|reason| error(400, reason))?;
    management.subscriptions.clear();
    Ok(subscription)
}

async fn list_subscriptions(config: &CompiledConfig, client: &Client) -> Reply {
    let mut providers = serde_json::Map::new();
    let mut warning = None;
    match client.json("/providers/proxies").await {
        Ok(value) => {
            if let Some(map) = value
                .get("providers")
                .and_then(serde_json::Value::as_object)
            {
                providers = map.clone();
            }
        }
        Err(reason) => warning = Some(reason),
    }
    let rows = config
        .settings
        .singbox
        .subscriptions
        .iter()
        .map(|subscription| {
            let provider = providers.get(&subscription.name);
            SubscriptionView {
                name: &subscription.name,
                url: &subscription.url,
                update_interval_hours: subscription.update_interval_hours,
                enabled: subscription.enabled,
                use_proxy: subscription.use_proxy,
                updated_at: provider.and_then(|item| item.get("updatedAt")).cloned(),
                node_count: provider
                    .and_then(|item| item.get("proxies"))
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::len),
            }
        })
        .collect::<Vec<_>>();
    success(serde_json::json!({
        "subscriptions": rows,
        "group": selector_tag(&serde_json::Value::Null, &config.settings.singbox, Backend::SingBox),
        "error": warning,
    }))
}

async fn list_nodes(config: &CompiledConfig, client: &Client) -> Reply {
    let Managed {
        revision,
        config: current,
        backend,
    } = match client.load_config().await {
        Ok(value) => value,
        Err(reason) => return error(502, reason),
    };
    let map = match client.proxy_details().await {
        Ok(value) => value,
        Err(reason) => return error(502, reason),
    };
    let groups = map.iter().filter(|(name, item)| {
        name.as_str() != "GLOBAL" && item.get("all").is_some()
    }).map(|(name, item)| {
        let now = item.get("now").and_then(serde_json::Value::as_str).unwrap_or_default();
        let nodes = item.get("all").and_then(serde_json::Value::as_array).into_iter().flatten()
            .filter_map(serde_json::Value::as_str).map(|node| {
                let info = map.get(node);
                let history = info.and_then(|item| item.get("history")).and_then(serde_json::Value::as_array)
                    .and_then(|history| history.last()).and_then(|entry| entry.get("delay")).cloned().unwrap_or(serde_json::Value::Null);
                serde_json::json!({
                    "name": node,
                    "type": info.and_then(|item| item.get("type")).cloned().unwrap_or_default(),
                    "source": node.split_once('/').map(|(provider, _)| provider),
                    "selected": node == now,
                    "udp": info.and_then(|item| item.get("udp")).and_then(serde_json::Value::as_bool).unwrap_or(false),
                    "delay": history,
                })
            }).collect::<Vec<_>>();
        serde_json::json!({
            "name": name,
            "type": item.get("type").cloned().unwrap_or_default(),
            "now": now,
            "nodes": nodes,
        })
    }).collect::<Vec<_>>();
    success(serde_json::json!({
        "groups": groups,
        "manual_nodes": current_manual_nodes(&current, &config.settings.singbox, backend),
        "revision": revision,
        "default_group": selector_tag(&current, &config.settings.singbox, backend),
        "backend": backend.name(),
    }))
}

async fn mutate_subscription(
    control: Option<&RuntimeControl>,
    config: &CompiledConfig,
    method: &str,
    name: &str,
    body: &[u8],
) -> Reply {
    let Some(control) = control else {
        return error(503, "runtime control is unavailable");
    };
    let mut settings = config.settings.clone();
    let existing = settings
        .singbox
        .subscriptions
        .iter()
        .position(|item| item.name == name);
    match method {
        "POST" if name.is_empty() => {
            let subscription =
                match decode_input::<ProviderInput>(body).and_then(parse_subscription) {
                    Ok(value) => value,
                    Err(reply) => return reply,
                };
            if settings
                .singbox
                .subscriptions
                .iter()
                .any(|item| item.name == subscription.name)
            {
                return error(409, "同名订阅已存在");
            }
            settings.singbox.subscriptions.push(subscription);
        }
        "PUT" if !name.is_empty() => {
            let Some(index) = existing else {
                return error(404, "订阅不存在");
            };
            let subscription =
                match decode_input::<ProviderInput>(body).and_then(parse_subscription) {
                    Ok(value) => value,
                    Err(reply) => return reply,
                };
            if subscription.name != name {
                return error(400, "订阅名称是节点标签的一部分，请新建订阅后删除旧订阅");
            }
            settings.singbox.subscriptions[index] = subscription;
        }
        "DELETE" if !name.is_empty() => {
            let Some(index) = existing else {
                return error(404, "订阅不存在");
            };
            settings.singbox.subscriptions.remove(index);
        }
        _ => return error(405, "method not allowed"),
    }
    settings.singbox.providers_managed = true;
    let client = match Client::new(config) {
        Ok(client) => client,
        Err(reply) => return reply,
    };
    let mut name = "代理后端";
    if let Ok(managed) = client.load_config().await {
        settings.singbox.group = selector_tag(&managed.config, &settings.singbox, managed.backend);
        name = managed.backend.name();
    }
    match save_managed(control, config, settings.clone()).await {
        Ok(()) => success(
            serde_json::json!({"message": format!("{name} 订阅已保存并重载"), "subscriptions": subscriptions_json(&settings.singbox)}),
        ),
        Err(reply) => reply,
    }
}

async fn mutate_node(
    control: Option<&RuntimeControl>,
    config: &CompiledConfig,
    method: &str,
    tag: &str,
    body: &[u8],
) -> Reply {
    let Some(control) = control else {
        return error(503, "runtime control is unavailable");
    };
    let client = match Client::new(config) {
        Ok(client) => client,
        Err(reply) => return reply,
    };
    let Managed {
        config: current,
        backend,
        ..
    } = match client.load_config().await {
        Ok(value) => value,
        Err(reason) => return error(502, reason),
    };
    let mut settings = config.settings.clone();
    settings.singbox.group = selector_tag(&current, &settings.singbox, backend);
    let mut nodes = current_manual_nodes(&current, &settings.singbox, backend);
    let existing = nodes.iter().position(|item| item.tag == tag);
    if matches!(method, "PUT" | "DELETE") && !tag.is_empty() && existing.is_none() {
        return error(404, "手动节点不存在");
    }
    let input = if method == "POST" || method == "PUT" {
        match decode_input::<NodeInput>(body) {
            Ok(value) => Some(value),
            Err(reply) => return reply,
        }
    } else {
        None
    };
    let node = if let Some(input) = input {
        if method == "PUT" && input.link.trim().is_empty() {
            // Never display persisted credentials in an edit form. An empty
            // replacement link retains the complete native outbound.
            let mut node = nodes[existing.unwrap()].clone();
            node.enabled = input.enabled;
            node
        } else {
            match client.parse_node(&input, backend).await {
                Ok(mut node) => {
                    if let Err(reason) =
                        crate::config::settings::validate_singbox_tag(&node.tag, "node tag")
                    {
                        return error(400, reason);
                    }
                    node.enabled = input.enabled;
                    node
                }
                Err(reason) => return error(400, reason),
            }
        }
    } else {
        SingboxNode::default()
    };
    match method {
        "POST" if tag.is_empty() => {
            if native_names(&current, backend).contains(&node.tag) {
                return error(409, "同名节点或出口已存在");
            }
            nodes.push(node);
        }
        "PUT" if !tag.is_empty() => {
            if node.tag != tag {
                return error(400, "节点标识会用于规则绑定，不能在此处改名");
            }
            nodes[existing.unwrap()] = node;
        }
        "DELETE" if !tag.is_empty() => {
            nodes.remove(existing.unwrap());
        }
        _ => return error(405, "method not allowed"),
    }
    settings.singbox.nodes = nodes;
    settings.singbox.nodes_managed = true;
    match save_managed(control, config, settings).await {
        Ok(()) => {
            success(serde_json::json!({"message": format!("{} 节点已保存并重载", backend.name())}))
        }
        Err(reply) => reply,
    }
}

async fn select_node(config: &CompiledConfig, body: &[u8]) -> Reply {
    #[derive(Deserialize)]
    struct Select {
        group: String,
        node: String,
    }
    let select: Select = match decode_input(body) {
        Ok(value) => value,
        Err(reply) => return reply,
    };
    if select.group.is_empty() || select.node.is_empty() {
        return error(400, "出口组和节点不能为空");
    }
    let client = match Client::new(config) {
        Ok(client) => client,
        Err(reply) => return reply,
    };
    let payload = serde_json::to_vec(&serde_json::json!({"name": select.node})).unwrap_or_default();
    let path = format!("/proxies/{}", escape(&select.group));
    match client.request("PUT", &path, Some(payload), 10).await {
        Ok((204, _)) => success(
            serde_json::json!({"message": format!("{} 已切换到 {}", select.group, select.node)}),
        ),
        Ok((status, bytes)) => error(502, http_error("选择节点", status, &bytes)),
        Err(reason) => error(502, reason),
    }
}

async fn delay_node(config: &CompiledConfig, body: &[u8]) -> Reply {
    #[derive(Deserialize)]
    struct Delay {
        name: String,
        url: Option<String>,
        timeout: Option<u64>,
    }
    let delay: Delay = match decode_input(body) {
        Ok(value) => value,
        Err(reply) => return reply,
    };
    if delay.name.is_empty() {
        return error(400, "节点名称不能为空");
    }
    let client = match Client::new(config) {
        Ok(client) => client,
        Err(reply) => return reply,
    };
    let test_url = delay
        .url
        .unwrap_or_else(|| "https://cp.cloudflare.com/generate_204".to_owned());
    let timeout = delay.timeout.unwrap_or(5000).clamp(100, 15000);
    let path = format!(
        "/proxies/{}/delay?url={}&timeout={timeout}",
        escape(&delay.name),
        escape(&test_url)
    );
    match client.request("GET", &path, None, timeout / 1000 + 8).await {
        Ok((200, bytes)) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(mut value) => {
                value["name"] = serde_json::Value::String(delay.name);
                success(value)
            }
            Err(_) => error(502, "invalid delay response"),
        },
        Ok((status, bytes)) => success(
            serde_json::json!({"name": delay.name, "error": http_error("延迟测试", status, &bytes)}),
        ),
        Err(reason) => success(serde_json::json!({"name": delay.name, "error": reason})),
    }
}

async fn update_subscription(config: &CompiledConfig, name: &str) -> Reply {
    if !config
        .settings
        .singbox
        .subscriptions
        .iter()
        .any(|item| item.name == name && item.enabled)
    {
        return error(404, "启用的订阅不存在");
    }
    let client = match Client::new(config) {
        Ok(client) => client,
        Err(reply) => return reply,
    };
    match client
        .request(
            "PUT",
            &format!("/providers/proxies/{}", escape(name)),
            None,
            90,
        )
        .await
    {
        Ok((204, _)) => list_subscriptions(config, &client).await,
        Ok((status, bytes)) => error(502, http_error("更新订阅", status, &bytes)),
        Err(reason) => error(502, reason),
    }
}

pub(crate) async fn handle(
    control: Option<&RuntimeControl>,
    config: &CompiledConfig,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Reply> {
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix("/api/v1/singbox/")?;
    if rest == "outbounds" || body.len() > MAX_BODY {
        return None;
    }
    let reply = match rest {
        "subscriptions" if method == "GET" => match Client::new(config) {
            Ok(client) => list_subscriptions(config, &client).await,
            Err(reply) => reply,
        },
        "subscriptions" => mutate_subscription(control, config, method, "", body).await,
        "nodes" if method == "GET" => match Client::new(config) {
            Ok(client) => list_nodes(config, &client).await,
            Err(reply) => reply,
        },
        "nodes" => mutate_node(control, config, method, "", body).await,
        "select" if method == "POST" => select_node(config, body).await,
        "delay" if method == "POST" => delay_node(config, body).await,
        _ if rest.starts_with("subscriptions/") => {
            let tail = &rest["subscriptions/".len()..];
            if let Some(name) = tail.strip_suffix("/update") {
                if method == "POST" {
                    update_subscription(config, &decode(name)).await
                } else {
                    error(405, "method not allowed")
                }
            } else if tail.contains('/') {
                error(404, "not found")
            } else {
                mutate_subscription(control, config, method, &decode(tail), body).await
            }
        }
        _ if rest.starts_with("nodes/") && !rest["nodes/".len()..].contains('/') => {
            mutate_node(
                control,
                config,
                method,
                &decode(&rest["nodes/".len()..]),
                body,
            )
            .await
        }
        _ => return None,
    };
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> serde_json::Value {
        serde_json::json!({
            "outbounds": [
                {"type":"selector","tag":"proxy","outbounds":["proxy-main"],"providers":[],"default":"proxy-main"},
                {"type":"socks","tag":"proxy-main","server":"10.0.0.4","server_port":2000,"version":"5"},
                {"type":"socks","tag":"proxy-alt","server":"10.0.0.4","server_port":2020,"version":"5"}
            ]
        })
    }

    #[test]
    fn compiles_native_providers_and_preserves_manual_nodes() {
        let mut settings = SingboxManagement::default();
        settings.providers_managed = true;
        settings.group = "proxy".to_owned();
        settings.subscriptions.push(SingboxSubscription {
            name: "airport".to_owned(),
            url: "https://example.com/sub".to_owned(),
            update_interval_hours: 12,
            enabled: true,
            use_proxy: true,
        });
        let next = managed_config(&config(), &settings, Backend::SingBox).unwrap();
        assert_eq!(next["providers"][0]["type"], "remote");
        assert_eq!(next["providers"][0]["update_interval"], "12h");
        assert_eq!(next["providers"][0]["http_client"]["detour"], "proxy");
        let mut direct = SingboxManagement::default();
        direct.providers_managed = true;
        direct.group = "proxy".to_owned();
        direct.subscriptions.push(SingboxSubscription {
            name: "direct-provider".to_owned(),
            url: "https://example.com/sub".to_owned(),
            update_interval_hours: 1,
            enabled: true,
            use_proxy: false,
        });
        let direct_next = managed_config(&config(), &direct, Backend::SingBox).unwrap();
        assert_eq!(
            direct_next["providers"][0]["http_client"],
            serde_json::json!({})
        );
        let selector = next["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["tag"] == "proxy")
            .unwrap();
        assert_eq!(selector["providers"][0], "airport");
        assert_eq!(selector["outbounds"].as_array().unwrap().len(), 2);
        assert_eq!(selector["default"], "proxy-main");
    }

    #[test]
    fn managed_nodes_preserve_selector_and_drop_deleted_default() {
        let mut settings = SingboxManagement::default();
        settings.nodes_managed = true;
        settings.nodes.push(SingboxNode {
            tag: "new-node".to_owned(),
            kind: "socks".to_owned(),
            options: serde_json::Map::from_iter([
                ("server".to_owned(), serde_json::json!("192.0.2.1")),
                ("server_port".to_owned(), serde_json::json!(1080)),
            ]),
            enabled: true,
        });
        let next = managed_config(&config(), &settings, Backend::SingBox).unwrap();
        let socks: Vec<_> = next["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "socks")
            .collect();
        assert_eq!(socks.len(), 1);
        assert_eq!(socks[0]["tag"], "new-node");
        let selector = next["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["tag"] == "proxy")
            .unwrap();
        assert_eq!(selector["default"], "new-node");
        assert_eq!(selector["outbounds"][0], "new-node");
        let mut deleted = settings.clone();
        deleted.nodes.clear();
        let compiled = managed_config(&next, &deleted, Backend::SingBox).unwrap();
        assert!(
            !compiled["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "socks")
        );
        let selector = compiled["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["tag"] == "proxy")
            .unwrap();
        assert_eq!(selector["outbounds"], serde_json::json!(["direct"]));
        assert_eq!(selector["default"], "direct");
    }

    #[test]
    fn generic_outbound_options_survive_management_round_trip() {
        let native = serde_json::json!({
            "type": "vless", "tag": "vless-main", "server": "192.0.2.10",
            "server_port": 443, "uuid": "11111111-1111-1111-1111-111111111111",
            "flow": "xtls-rprx-vision",
            "tls": {"enabled": true, "server_name": "example.com"},
            "transport": {"type": "ws", "path": "/proxy"}
        });
        let node = SingboxNode::from_outbound(&native).unwrap();
        assert_eq!(node.outbound(), native);
        let decoded: SingboxNode =
            serde_json::from_value(serde_json::to_value(&node).unwrap()).unwrap();
        assert_eq!(decoded, node);
        let mut management = SingboxManagement {
            nodes_managed: true,
            nodes: vec![node],
            ..SingboxManagement::default()
        };
        management.group = "proxy".to_owned();
        let mut current = config();
        current["outbounds"].as_array_mut().unwrap().push(native);
        let compiled = managed_config(&current, &management, Backend::SingBox).unwrap();
        let saved = compiled["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["tag"] == "vless-main")
            .unwrap();
        assert_eq!(saved["tls"]["server_name"], "example.com");
        assert_eq!(saved["transport"]["path"], "/proxy");
        assert_eq!(saved["flow"], "xtls-rprx-vision");
        let selector = compiled["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["tag"] == "proxy")
            .unwrap();
        assert!(
            selector["outbounds"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("vless-main"))
        );
    }

    fn mihomo_config() -> serde_json::Value {
        serde_json::json!({
            "dns": {"enable": true, "enhanced-mode": "fake-ip"},
            "proxies": [{"name": "old", "type": "socks5", "server": "10.0.0.4", "port": 2000}],
            "proxy-groups": [{"name": "proxy", "type": "select", "proxies": ["old"]}],
            "rules": ["MATCH,proxy"]
        })
    }

    fn mihomo_node(tag: &str) -> SingboxNode {
        SingboxNode::from_mihomo_proxy(&serde_json::json!({
            "name": tag, "type": "vless", "server": "192.0.2.20", "port": 443,
            "uuid": "00000000-0000-0000-0000-000000000000",
            "ws-opts": {"path": "/ws"}
        }))
        .unwrap()
    }

    #[test]
    fn mihomo_owns_proxies_providers_and_group() {
        let mut management = SingboxManagement {
            providers_managed: true,
            nodes_managed: true,
            nodes: vec![mihomo_node("vless-main")],
            ..SingboxManagement::default()
        };
        management.subscriptions.push(SingboxSubscription {
            name: "airport".to_owned(),
            url: "https://example.com/sub".to_owned(),
            update_interval_hours: 12,
            enabled: true,
            use_proxy: true,
        });
        management.subscriptions.push(SingboxSubscription {
            name: "off".to_owned(),
            url: "https://example.com/off".to_owned(),
            update_interval_hours: 1,
            enabled: false,
            use_proxy: false,
        });
        let next = managed_config(&mihomo_config(), &management, Backend::Mihomo).unwrap();
        assert_eq!(
            next["proxies"],
            serde_json::json!([{
                "name": "vless-main", "type": "vless", "server": "192.0.2.20", "port": 443,
                "uuid": "00000000-0000-0000-0000-000000000000", "ws-opts": {"path": "/ws"}
            }])
        );
        let provider = &next["proxy-providers"]["airport"];
        assert_eq!(provider["type"], "http");
        assert_eq!(provider["interval"], 43200);
        assert_eq!(provider["proxy"], "proxy");
        assert_eq!(provider["override"]["additional-prefix"], "airport/");
        assert!(next["proxy-providers"].get("off").is_none());
        assert_eq!(
            next["proxy-groups"],
            serde_json::json!([{"name": "proxy", "type": "select", "proxies": ["vless-main"], "use": ["airport"]}])
        );
        // Unmanaged sections are kept.
        assert_eq!(next["rules"], mihomo_config()["rules"]);
        assert_eq!(next["dns"], mihomo_config()["dns"]);
    }

    #[test]
    fn mihomo_group_falls_back_to_direct_and_providers_download_directly() {
        let management = SingboxManagement {
            nodes_managed: true,
            ..SingboxManagement::default()
        };
        let next = managed_config(&mihomo_config(), &management, Backend::Mihomo).unwrap();
        assert_eq!(next["proxies"], serde_json::json!([]));
        assert_eq!(
            next["proxy-groups"][0]["proxies"],
            serde_json::json!(["DIRECT"])
        );

        // Only provider nodes: no DIRECT member, and the first download
        // cannot go through a group that only the provider fills.
        let mut providers_only = management.clone();
        providers_only.providers_managed = true;
        providers_only.subscriptions.push(SingboxSubscription {
            name: "airport".to_owned(),
            url: "https://example.com/sub".to_owned(),
            update_interval_hours: 24,
            enabled: true,
            use_proxy: true,
        });
        let next = managed_config(&mihomo_config(), &providers_only, Backend::Mihomo).unwrap();
        assert!(next["proxy-groups"][0].get("proxies").is_none());
        assert_eq!(
            next["proxy-groups"][0]["use"],
            serde_json::json!(["airport"])
        );
        assert_eq!(next["proxy-providers"]["airport"]["proxy"], "DIRECT");
    }

    #[test]
    fn mihomo_reads_manual_nodes_groups_and_reserved_names() {
        let config = mihomo_config();
        let nodes = current_manual_nodes(&config, &SingboxManagement::default(), Backend::Mihomo);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].tag, "old");
        assert_eq!(nodes[0].mihomo_proxy()["port"], 2000);
        assert_eq!(
            selector_tag(&config, &SingboxManagement::default(), Backend::Mihomo),
            "proxy"
        );
        let names = native_names(&config, Backend::Mihomo);
        for name in ["old", "proxy", "DIRECT", "REJECT"] {
            assert!(names.iter().any(|item| item == name), "{name}");
        }
    }
}
