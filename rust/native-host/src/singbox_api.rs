//! sing-box subscription and node management for the AtomDNS WebUI.
//!
//! The WebUI owns its subscription list in `webinfo/settings.json` and compiles
//! it to native sing-box `remote` providers plus a provider-aware selector.
//! Node selection, latency tests and provider updates use the live Clash API.

use serde::{Deserialize, Serialize};

use crate::config::CompiledConfig;
use crate::config::settings::{Settings, SingboxManagement, SingboxNode, SingboxSubscription};
use crate::rule_sources::Reply;
use crate::runtime_snapshot::{ApplyFailure, RuntimeControl};

const MAX_BODY: usize = 16 * 1024 * 1024;

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
                "请先在 上游设置 › 代理与 ECS 配置 sing-box API 地址与密钥",
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
            return Err(http_error("sing-box API", status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|_| "sing-box returned invalid JSON".to_owned())
    }

    async fn load_config(&self) -> Result<(String, serde_json::Value), String> {
        let value = self.json("/atomdns/config").await?;
        let revision = value
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let config = value.get("config").cloned().unwrap_or_default();
        if revision.is_empty() || !config.is_object() {
            return Err(
                "sing-box managed configuration is unavailable; use one writable config file"
                    .to_owned(),
            );
        }
        Ok((revision, config))
    }

    async fn put_config(&self, revision: &str, config: &serde_json::Value) -> Result<(), String> {
        let body = serde_json::to_vec(&serde_json::json!({"revision": revision, "config": config}))
            .map_err(|error| error.to_string())?;
        let (status, bytes) = self
            .request("PUT", "/atomdns/config", Some(body), 120)
            .await?;
        if status != 200 {
            return Err(http_error("sing-box configuration", status, &bytes));
        }
        let accepted: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| "sing-box returned an invalid save response".to_owned())?;
        let next_revision = accepted
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if next_revision.is_empty() {
            return Err("sing-box did not return a configuration revision".to_owned());
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
                    .map_err(|_| "sing-box returned an invalid reload status".to_owned())?;
                if state.get("revision").and_then(serde_json::Value::as_str) == Some(next_revision)
                    && state.get("reloading").and_then(serde_json::Value::as_bool) == Some(false)
                {
                    if let Some(reason) = state.get("error").and_then(serde_json::Value::as_str) {
                        return Err(format!("sing-box 配置应用失败，已恢复旧配置: {reason}"));
                    }
                    return Ok(());
                }
            } else if status != 503 {
                return Err(http_error("sing-box reload status", status, &bytes));
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err("等待 sing-box 应用配置超时，请检查 sing-box 状态后重试".to_owned())
    }

    async fn parse_node(&self, input: &NodeInput) -> Result<SingboxNode, String> {
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
            .map_err(|_| "sing-box returned an invalid node".to_owned())?;
        SingboxNode::from_outbound(&value)
            .ok_or_else(|| "sing-box returned an incomplete node".to_owned())
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

fn current_manual_nodes(
    config: &serde_json::Value,
    management: &SingboxManagement,
) -> Vec<SingboxNode> {
    if management.nodes_managed {
        return management.nodes.clone();
    }
    config
        .get("outbounds")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| !is_group_or_builtin(item))
        .filter_map(SingboxNode::from_outbound)
        .collect()
}

fn selector_tag(config: &serde_json::Value, management: &SingboxManagement) -> String {
    if !management.group.is_empty() {
        return management.group.clone();
    }
    config
        .get("outbounds")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| {
            item.get("type").and_then(serde_json::Value::as_str) == Some("selector")
                && item.get("tag").and_then(serde_json::Value::as_str) == Some("proxy")
        })
        .or_else(|| {
            config
                .get("outbounds")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .find(|item| {
                    item.get("type").and_then(serde_json::Value::as_str) == Some("selector")
                })
        })
        .and_then(|item| item.get("tag").and_then(serde_json::Value::as_str))
        .unwrap_or("proxy")
        .to_owned()
}

/// Applies the WebUI-owned pieces while preserving unrelated sing-box JSON.
fn managed_config(
    current: &serde_json::Value,
    management: &SingboxManagement,
) -> Result<serde_json::Value, String> {
    let mut result = current.clone();
    let object = result
        .as_object_mut()
        .ok_or("invalid sing-box configuration")?;
    let group = selector_tag(current, management);
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

/// The settings and sing-box configuration are a two-resource transaction:
/// validation and a config write precede a reload; failures restore the prior
/// settings or prior sing-box JSON rather than silently retaining a split state.
async fn save_managed(
    control: &RuntimeControl,
    config: &CompiledConfig,
    settings: Settings,
) -> Result<(), Reply> {
    let client = Client::new(config)?;
    let (revision, current) = client
        .load_config()
        .await
        .map_err(|reason| error(502, reason))?;
    let next = managed_config(&current, &settings.singbox).map_err(|reason| error(400, reason))?;
    if next == current {
        return control
            .apply_settings(settings)
            .await
            .map(|_| ())
            .map_err(apply_error);
    }
    // Validate and persist sing-box first. Only then persist AtomDNS settings,
    // so a rejected provider/node cannot leave the WebUI showing a subscription
    // that never loaded. If the second step fails, restore sing-box too.
    if let Err(reason) = client.put_config(&revision, &next).await {
        return Err(error(502, reason));
    }
    if let Err(failure) = control.apply_settings(settings).await {
        let (latest_revision, _) = client.load_config().await.map_err(|reason| {
            error(
                502,
                format!("AtomDNS 设置未应用: {failure}; sing-box revision read failed: {reason}"),
            )
        })?;
        let restored = client.put_config(&latest_revision, &current).await;
        return Err(error(
            502,
            match restored {
                Ok(()) => format!("AtomDNS 设置未应用，已恢复 sing-box 配置: {failure}"),
                Err(reason) => {
                    format!("AtomDNS 设置未应用: {failure}; sing-box 配置恢复也失败: {reason}")
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
        "group": selector_tag(&serde_json::Value::Null, &config.settings.singbox),
        "error": warning,
    }))
}

async fn list_nodes(config: &CompiledConfig, client: &Client) -> Reply {
    let (revision, current) = match client.load_config().await {
        Ok(value) => value,
        Err(reason) => return error(502, reason),
    };
    let proxies = match client.json("/proxies").await {
        Ok(value) => value,
        Err(reason) => return error(502, reason),
    };
    let map = proxies
        .get("proxies")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
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
        "manual_nodes": current_manual_nodes(&current, &config.settings.singbox),
        "revision": revision,
        "default_group": selector_tag(&current, &config.settings.singbox),
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
    if let Ok((_, current)) = client.load_config().await {
        settings.singbox.group = selector_tag(&current, &settings.singbox);
    }
    match save_managed(control, config, settings.clone()).await {
        Ok(()) => success(
            serde_json::json!({"message": "sing-box 订阅已保存并重载", "subscriptions": subscriptions_json(&settings.singbox)}),
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
    let (_, current) = match client.load_config().await {
        Ok(value) => value,
        Err(reason) => return error(502, reason),
    };
    let mut settings = config.settings.clone();
    settings.singbox.group = selector_tag(&current, &settings.singbox);
    let mut nodes = current_manual_nodes(&current, &settings.singbox);
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
            match client.parse_node(&input).await {
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
            let duplicate = current
                .get("outbounds")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .any(|item| {
                    item.get("tag").and_then(serde_json::Value::as_str) == Some(node.tag.as_str())
                });
            if duplicate {
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
        Ok(()) => success(serde_json::json!({"message": "sing-box 节点已保存并重载"})),
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
        let next = managed_config(&config(), &settings).unwrap();
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
        let direct_next = managed_config(&config(), &direct).unwrap();
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
        let next = managed_config(&config(), &settings).unwrap();
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
        let compiled = managed_config(&next, &deleted).unwrap();
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
        let compiled = managed_config(&current, &management).unwrap();
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
}
