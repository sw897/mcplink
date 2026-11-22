//! Aggregate views reference managed connections; they never own upstream credentials.
use crate::{
    Engine,
    catalog::{CatalogKind, CollectedCatalog},
    engine::required,
    model::{Connection, Definition, GatewayMember, digest},
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

const MAX_ITEMS: usize = 10_000;
const PAGE_SIZE: usize = 100;

const CATALOG_TTL: Duration = Duration::from_secs(300);
const CATALOG_BUDGET: usize = 16 * 1024 * 1024;

pub(crate) struct CatalogCache {
    entries: Mutex<HashMap<String, Arc<CatalogSnapshot>>>,
    epoch: String,
}

impl Default for CatalogCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            epoch: uuid::Uuid::new_v4().to_string(),
        }
    }
}

struct CatalogSnapshot {
    id: String,
    entry_id: String,
    entry_revision: u64,
    members: String,
    method: String,
    rows: Vec<Value>,
    errors: Vec<Value>,
    created: Instant,
    bytes: usize,
}

impl CatalogCache {
    fn get(&self, id: &str) -> Option<Arc<CatalogSnapshot>> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, value| value.created.elapsed() < CATALOG_TTL);
        entries.get(id).cloned()
    }

    fn insert(&self, snapshot: Arc<CatalogSnapshot>) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, value| value.created.elapsed() < CATALOG_TTL);
        while entries.len() >= 32
            || entries.values().map(|s| s.bytes).sum::<usize>() + snapshot.bytes > CATALOG_BUDGET
        {
            let oldest = entries
                .iter()
                .min_by_key(|(_, s)| s.created)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                entries.remove(&id);
            } else {
                break;
            }
        }
        entries.insert(snapshot.id.clone(), snapshot);
    }
}

fn member_signature(engine: &Engine, entry: &Connection, kind: CatalogKind) -> Result<String> {
    let index = kind.generation_index();
    let mut signature = vec![json!(engine.pool.catalog_generation(&entry.id)[index])];
    for member in entry.members.as_ref().context("不是聚合入口")? {
        signature.push(member_stamp(engine, member, index));
    }
    Ok(digest(&serde_json::to_vec(&signature)?))
}

fn member_stamp(engine: &Engine, member: &GatewayMember, index: usize) -> Value {
    match engine.store.connection(&member.connection_id) {
        Ok(connection) => json!([
            connection.id,
            connection.revision,
            connection.active,
            engine.pool.catalog_generation(&connection.id)[index]
        ]),
        Err(_) => json!([member.connection_id, "unavailable"]),
    }
}

fn catalog_page(snapshot: &CatalogSnapshot, field: &str, offset: usize) -> Result<Value> {
    let end = (offset + PAGE_SIZE).min(snapshot.rows.len());
    let mut result = json!({"_meta":{"mcplink":{"upstreamErrors":snapshot.errors}}});
    result[field] = json!(&snapshot.rows[offset..end]);
    if end < snapshot.rows.len() {
        result["nextCursor"] = json!(URL_SAFE_NO_PAD.encode(serde_json::to_vec(
            &json!({"snapshot":snapshot.id,"offset":end})
        )?));
    }
    Ok(result)
}

pub fn description() -> Value {
    json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{},"resources":{},"prompts":{}},
        "serverInfo":{"name":"mcplink","version":env!("CARGO_PKG_VERSION")},
        "instructions":"Tools, resources and prompts are namespaced by managed service. List-change notifications are sent when the entry or its upstream catalogs change. Clients without live refresh must reload MCP."})
}

/// Cheap scoped fingerprints: no upstream discovery, Vault access or business calls.
pub fn catalog_version(engine: &Engine, id: &str) -> Result<Value> {
    let entry = engine.store.connection(id)?;
    let mut fingerprints = Vec::new();
    for (index, family) in ["tools", "resources", "prompts"].iter().enumerate() {
        let mut rows = vec![json!([
            engine.catalog.epoch,
            entry.id,
            entry.revision,
            entry.active,
            engine.paused.load(Ordering::SeqCst),
            engine.pool.catalog_generation(id)[index]
        ])];
        if let Some(members) = &entry.members {
            rows.extend(
                members
                    .iter()
                    .map(|member| member_stamp(engine, member, index)),
            );
        }
        fingerprints.push((
            family.to_string(),
            json!(digest(&serde_json::to_vec(&rows)?)),
        ));
    }
    Ok(Value::Object(fingerprints.into_iter().collect()))
}

pub fn save(engine: &Engine, p: &Value) -> Result<Value> {
    ensure!(
        p.as_object().is_some_and(|m| m.keys().all(|k| matches!(
            k.as_str(),
            "id" | "editVersion" | "title" | "connectionIds" | "toolSelections"
        ))),
        "统一入口包含未知字段"
    );
    let previous = p
        .get("id")
        .map(|id| id.as_str().context("入口标识无效"))
        .transpose()?
        .map(|id| engine.store.connection(id))
        .transpose()?;
    if let Some(old) = &previous {
        ensure!(old.members.is_some(), "此连接不是聚合入口");
        ensure!(
            p.get("editVersion").and_then(Value::as_str) == Some(old.edit_version().as_str()),
            "入口已更新，请重新打开服务选择"
        );
    }
    let ids = p
        .get("connectionIds")
        .and_then(Value::as_array)
        .context("请选择托管服务，空数组表示不提供任何服务")?;
    ensure!(ids.len() <= 64, "每个入口最多选择 64 项服务");
    let selections = p
        .get("toolSelections")
        .map(|v| {
            v.as_object()
                .context("工具范围须为服务 ID 到工具列表的映射")
        })
        .transpose()?;
    let mut seen = HashSet::new();
    let mut namespaces = HashSet::new();
    let mut members = Vec::new();
    for id in ids {
        let id = id.as_str().context("服务标识无效")?;
        ensure!(seen.insert(id), "服务选择中存在重复连接");
        let upstream = engine.store.connection(id)?;
        ensure!(
            upstream.members.is_none(),
            "聚合入口只能选择上游服务，不能嵌套入口"
        );
        let old_namespace = previous
            .as_ref()
            .and_then(|c| c.members.as_ref())
            .and_then(|m| m.iter().find(|m| m.connection_id == id))
            .map(|m| m.namespace.clone());
        let namespace = old_namespace.unwrap_or_else(|| {
            let name: String = upstream
                .name
                .chars()
                .take(24)
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("s_{}_{}", name, &digest(id.as_bytes())[..12])
        });
        ensure!(
            namespaces.insert(namespace.clone()),
            "入口服务命名空间冲突，请重新创建入口"
        );
        let include_tools = if let Some(value) = selections.and_then(|s| s.get(id)) {
            if value.is_null() {
                None
            } else {
                let values = value
                    .as_array()
                    .context("工具范围须为工具名数组，或 null 表示沿用服务范围")?;
                ensure!(values.len() <= MAX_ITEMS, "工具范围超过限制");
                let mut tools = Vec::new();
                let mut unique = HashSet::new();
                for value in values {
                    let name = value.as_str().context("工具名称无效")?;
                    ensure!(
                        !name.is_empty()
                            && name.len() <= 1024
                            && !name.chars().any(char::is_control)
                            && unique.insert(name),
                        "工具名称为空、重复或超出限制"
                    );
                    tools.push(name.to_owned());
                }
                tools.sort();
                Some(tools)
            }
        } else {
            previous
                .as_ref()
                .and_then(|c| c.members.as_ref())
                .and_then(|m| m.iter().find(|m| m.connection_id == id))
                .and_then(|m| m.include_tools.clone())
        };
        members.push(GatewayMember {
            connection_id: id.into(),
            namespace,
            include_tools,
        });
    }
    ensure!(
        selections.is_none_or(|s| s.keys().all(|id| seen.contains(id.as_str()))),
        "工具范围包含未选服务"
    );
    let title = p
        .get("title")
        .map(|v| v.as_str().context("入口名称无效"))
        .transpose()?
        .or_else(|| previous.as_ref().map(|c| c.title.as_str()))
        .unwrap_or("mcplink")
        .trim();
    ensure!(
        !title.is_empty() && title.chars().count() <= 120 && !title.chars().any(char::is_control),
        "入口名称应为 1 至 120 个字符"
    );
    if let Some(mut entry) = previous.clone()
        && entry.members.as_ref() == Some(&members)
    {
        if entry.title != title {
            entry.title = title.into();
            entry.edit_revision = entry
                .edit_revision
                .checked_add(1)
                .context("入口编辑版本超出范围")?;
            engine.store.save_connection(&entry)?;
        }
        return Ok(entry.public_value());
    }
    let mut entry = previous.clone().unwrap_or_else(|| Connection {
        id: uuid::Uuid::new_v4().to_string(),
        name: "mcplink".into(),
        title: title.into(),
        transport: "aggregate".into(),
        endpoint: String::new(),
        account_label: "使用上游托管账号".into(),
        auth_kind: "gateway".into(),
        definition_ref: String::new(),
        revision: 0,
        edit_revision: 0,
        active: true,
        status: "ready".into(),
        checked_at: None,
        tool_count: None,
        last_error: None,
        members: None,
    });
    entry.title = title.into();
    entry.members = Some(members);
    entry.tool_count = None;
    entry.checked_at = None;
    entry.status = if entry.active {
        "ready"
    } else {
        "disconnected"
    }
    .into();
    entry.last_error = None;
    // Existing connection persistence keeps the update atomic with respect to readers.
    // The definition contains no upstream address, secret or OAuth grant.
    crate::lifecycle::persist_definition(
        engine,
        &mut entry,
        &Definition {
            enabled: true,
            ..Definition::default()
        },
        None,
        false,
    )?;
    Ok(entry.public_value())
}

fn alias(namespace: &str, name: &str) -> String {
    let candidate = format!("{namespace}__{name}");
    if candidate.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        && !name.is_empty()
    {
        candidate
    } else {
        format!("{namespace}__item_{}", digest(name.as_bytes()))
    }
}

fn uri(namespace: &str, original: &str) -> String {
    // Prefixing keeps RFC6570 placeholders and their expansion unchanged.
    format!("mcplink://{namespace}/{original}")
}

fn content_uris(value: &mut Value, namespace: &str) {
    match value.get("type").and_then(Value::as_str) {
        Some("resource_link") => {
            if let Some(original) = value.get("uri").and_then(Value::as_str) {
                value["uri"] = json!(uri(namespace, original));
            }
        }
        Some("resource") => {
            if let Some(original) = value.pointer("/resource/uri").and_then(Value::as_str) {
                value["resource"]["uri"] = json!(uri(namespace, original));
            }
        }
        _ => {}
    }
}

fn rewrite_result(result: &mut Value, namespace: &str) {
    if let Some(items) = result.get_mut("contents").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(original) = item.get("uri").and_then(Value::as_str) {
                item["uri"] = json!(uri(namespace, original));
            }
        }
    }
    if let Some(items) = result.get_mut("content").and_then(Value::as_array_mut) {
        for item in items {
            content_uris(item, namespace);
        }
    }
    if let Some(items) = result.get_mut("messages").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(content) = item.get_mut("content") {
                content_uris(content, namespace);
            }
        }
    }
}

fn current(engine: &Engine, entry: &Connection) -> Result<()> {
    let now = engine.store.connection(&entry.id)?;
    ensure!(
        now.active && now.revision == entry.revision,
        "入口已更新或停用，请刷新服务清单"
    );
    ensure!(
        !engine.paused.load(Ordering::SeqCst),
        "共享后台已暂停，请在 mcplink 恢复"
    );
    Ok(())
}

#[derive(Clone)]
pub(crate) struct ToolRoute {
    pub connection_id: String,
    pub revision: u64,
    pub namespace: String,
    pub original_name: String,
}

impl ToolRoute {
    pub fn public_metadata(&self, tool: &mut Value) {
        if !tool.get("_meta").is_some_and(Value::is_object) {
            tool["_meta"] = json!({});
        }
        tool["_meta"]["mcplink"] = json!({"upstreamRevision": self.revision});
    }
}

pub(crate) struct EntryCatalog {
    pub rows: Vec<Value>,
    pub routes: HashMap<String, ToolRoute>,
    pub errors: Vec<Value>,
    signature: String,
    bytes: usize,
}

fn public_rows(rows: &[Value], routes: &HashMap<String, ToolRoute>) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            if let Some(route) = row["name"].as_str().and_then(|name| routes.get(name)) {
                route.public_metadata(&mut row);
            }
            row
        })
        .collect()
}

async fn collect_member(
    engine: &Arc<Engine>,
    member: &GatewayMember,
    kind: CatalogKind,
) -> Result<CollectedCatalog> {
    let id = &member.connection_id;
    tokio::time::timeout(Duration::from_secs(10), async {
        let connection = engine.store.connection(id)?;
        let supported = if kind == CatalogKind::Tools {
            member
                .include_tools
                .as_ref()
                .is_none_or(|tools| !tools.is_empty())
        } else {
            let def = engine.definition(&connection)?;
            if def.http_api.is_some()
                || (kind == CatalogKind::Prompts && def.prompts_disabled)
                || (kind != CatalogKind::Prompts && def.resources_disabled)
            {
                false
            } else {
                let (peer, _) = engine
                    .pool
                    .peer(engine.clone(), id, connection.revision)
                    .await?;
                peer.peer_info().is_some_and(|info| {
                    if kind == CatalogKind::Prompts {
                        info.capabilities.prompts.is_some()
                    } else {
                        info.capabilities.resources.is_some()
                    }
                })
            }
        };
        if !supported {
            return Ok(CollectedCatalog {
                revision: connection.revision,
                items: Vec::new(),
                diagnostics: Vec::new(),
            });
        }
        crate::catalog::collect(engine.clone(), &engine.pool, id, kind).await
    })
    .await
    .map_err(|_| crate::mcp::ConnectionFailure::Timeout.into())
    .and_then(|result| result)
}

pub(crate) async fn collect_entry(
    engine: Arc<Engine>,
    entry: &Connection,
    kind: crate::catalog::CatalogKind,
) -> Result<EntryCatalog> {
    current(&engine, entry)?;
    let (field, key) = (kind.field(), kind.key());
    let signature = member_signature(&engine, entry, kind)?;
    let mut jobs = tokio::task::JoinSet::new();
    for (index, member) in entry
        .members
        .as_ref()
        .context("不是聚合入口")?
        .iter()
        .cloned()
        .enumerate()
    {
        let engine = engine.clone();
        jobs.spawn(async move {
            let result = collect_member(&engine, &member, kind)
                .await
                .map_err(|error| {
                    crate::catalog::discovery_diagnostic(&engine, &member.connection_id, &error)
                });
            (index, member, result)
        });
    }
    let mut results = Vec::new();
    while let Some(result) = jobs.join_next().await {
        results.push(result.context("聚合清单任务失败")?);
    }
    results.sort_by_key(|(index, _, _)| *index);
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut routes = HashMap::new();
    let mut names = HashSet::new();
    for (_, member, result) in results {
        let catalog = match result {
            Ok(catalog) => catalog,
            Err(mut diagnostic) => {
                diagnostic["connectionId"] = json!(member.connection_id);
                diagnostic["namespace"] = json!(member.namespace);
                errors.push(diagnostic);
                continue;
            }
        };
        let revision = catalog.revision;
        for mut item in catalog.items {
            let original = item[key].as_str().context("上游清单标识无效")?.to_owned();
            if field == "tools" && !member.tool_allowed(&original) {
                continue;
            }
            let mapped = if key == "name" {
                alias(&member.namespace, &original)
            } else {
                uri(&member.namespace, &original)
            };
            ensure!(names.insert(mapped.clone()), "聚合清单名称冲突");
            if kind == crate::catalog::CatalogKind::Tools {
                routes.insert(
                    mapped.clone(),
                    ToolRoute {
                        connection_id: member.connection_id.clone(),
                        revision,
                        namespace: member.namespace.clone(),
                        original_name: original,
                    },
                );
            }
            item[key] = json!(mapped);
            rows.push(item);
            ensure!(
                rows.len() <= MAX_ITEMS,
                "聚合清单超过 10000 项，请减少入口服务选择"
            );
        }
    }
    current(&engine, entry)?;
    let serialized = serde_json::to_vec(&public_rows(&rows, &routes))?;
    ensure!(
        serialized.len() <= 4 * 1024 * 1024,
        "聚合清单过大，请减少入口服务选择"
    );
    ensure!(
        member_signature(&engine, entry, kind)? == signature,
        "入口成员已变化，请重新发现清单"
    );
    Ok(EntryCatalog {
        rows,
        routes,
        errors,
        signature,
        bytes: serialized.len(),
    })
}

async fn list(
    engine: Arc<Engine>,
    entry: &Connection,
    method: &str,
    params: &Value,
) -> Result<Value> {
    let kind = crate::catalog::CatalogKind::from_method(method).context("不支持的清单")?;
    let field = kind.field();
    if let Some(cursor) = params.get("cursor").filter(|v| !v.is_null()) {
        let signature = member_signature(&engine, entry, kind)?;
        let encoded = cursor
            .as_str()
            .filter(|c| c.len() < 1024)
            .context("聚合分页游标无效")?;
        let decoded: Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(encoded)
                .context("聚合分页游标无效")?,
        )
        .context("聚合分页游标无效")?;
        let snapshot = engine
            .catalog
            .get(required(&decoded, "snapshot")?)
            .context("聚合分页已过期，请从第一页刷新")?;
        ensure!(
            snapshot.entry_id == entry.id
                && snapshot.entry_revision == entry.revision
                && snapshot.method == method
                && snapshot.members == signature,
            "入口或成员已变化，请从第一页刷新"
        );
        let offset = decoded["offset"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0 && *v < snapshot.rows.len() && *v % PAGE_SIZE == 0)
            .context("聚合分页游标无效")?;
        return catalog_page(&snapshot, field, offset);
    }
    let catalog = collect_entry(engine.clone(), entry, kind).await?;
    let snapshot = Arc::new(CatalogSnapshot {
        id: uuid::Uuid::new_v4().to_string(),
        entry_id: entry.id.clone(),
        entry_revision: entry.revision,
        members: catalog.signature,
        method: method.into(),
        rows: public_rows(&catalog.rows, &catalog.routes),
        errors: catalog.errors,
        created: Instant::now(),
        bytes: catalog.bytes,
    });
    if snapshot.rows.len() > PAGE_SIZE {
        engine.catalog.insert(snapshot.clone());
    }
    catalog_page(&snapshot, field, 0)
}

enum RevisionPin {
    Exact(u64),
    Invalid,
}

async fn named_item(
    engine: &Arc<Engine>,
    entry: &Connection,
    kind: CatalogKind,
    target: &str,
    expected_route: Option<&ToolRoute>,
) -> Result<Option<(Value, ToolRoute)>> {
    current(engine, entry)?;
    let mut found = None;
    for member in entry.members.as_ref().context("不是聚合入口")? {
        if !target.starts_with(&format!("{}__", member.namespace))
            || expected_route.is_some_and(|route| {
                route.connection_id != member.connection_id || route.namespace != member.namespace
            })
        {
            continue;
        }
        let catalog = collect_member(engine, member, kind).await?;
        for mut item in catalog.items {
            let name = item["name"].as_str().context("上游能力名称无效")?;
            if alias(&member.namespace, name) != target
                || (kind == CatalogKind::Tools && !member.tool_allowed(name))
            {
                continue;
            }
            ensure!(found.is_none(), "聚合清单名称冲突，请调整入口服务选择");
            let route = ToolRoute {
                connection_id: member.connection_id.clone(),
                revision: catalog.revision,
                namespace: member.namespace.clone(),
                original_name: name.to_owned(),
            };
            item["name"] = json!(target);
            found = Some((item, route));
        }
    }
    current(engine, entry)?;
    Ok(found)
}

pub(crate) async fn find_tool(
    engine: &Arc<Engine>,
    entry: &Connection,
    name: &str,
) -> Result<Option<(Value, ToolRoute)>> {
    named_item(engine, entry, CatalogKind::Tools, name, None).await
}

fn resource_route(engine: &Engine, entry: &Connection, target: &str) -> Result<ToolRoute> {
    let mut candidates = entry
        .members
        .as_ref()
        .context("不是聚合入口")?
        .iter()
        .filter_map(|member| {
            target
                .strip_prefix(&format!("mcplink://{}/", member.namespace))
                .filter(|uri| !uri.is_empty())
                .map(|uri| (member, uri))
        });
    let (member, uri) = candidates
        .next()
        .context("此资源不在入口服务选择中，请刷新清单")?;
    ensure!(candidates.next().is_none(), "聚合资源路由冲突");
    Ok(ToolRoute {
        connection_id: member.connection_id.clone(),
        revision: engine.store.connection(&member.connection_id)?.revision,
        namespace: member.namespace.clone(),
        original_name: uri.into(),
    })
}

#[derive(Default)]
struct DispatchExpectation {
    revision: Option<RevisionPin>,
    schema_hash: Option<String>,
    route: Option<ToolRoute>,
}

pub(crate) async fn call_tool(
    engine: Arc<Engine>,
    entry: &Connection,
    route: ToolRoute,
    schema_hash: String,
    arguments: Value,
) -> Result<Value> {
    let request = json!({"method":"tools/call","params":{
        "name":alias(&route.namespace, &route.original_name),"arguments":arguments}});
    let expected = DispatchExpectation {
        revision: Some(RevisionPin::Exact(route.revision)),
        schema_hash: Some(schema_hash),
        route: Some(route),
    };
    dispatch(engine, entry, request, expected).await
}

pub fn request(
    engine: Arc<Engine>,
    entry: &Connection,
    request: Value,
) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + '_>> {
    let expected = DispatchExpectation {
        revision: request
            .pointer("/params/_meta/mcplink/expectedUpstreamRevision")
            .map(|value| {
                value
                    .as_u64()
                    .map(RevisionPin::Exact)
                    .unwrap_or(RevisionPin::Invalid)
            }),
        schema_hash: request
            .pointer("/params/_meta/mcplink/expectedSchemaHash")
            .and_then(Value::as_str)
            .map(str::to_owned),
        route: None,
    };
    dispatch(engine, entry, request, expected)
}

fn dispatch(
    engine: Arc<Engine>,
    entry: &Connection,
    request: Value,
    expected: DispatchExpectation,
) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + '_>> {
    Box::pin(async move {
        current(&engine, entry)?;
        let method = required(&request, "method")?;
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        if matches!(
            method,
            "tools/list" | "resources/list" | "resources/templates/list" | "prompts/list"
        ) {
            return list(engine, entry, method, &params).await;
        }
        let (key, route) = match method {
            "resources/read" => (
                "uri",
                resource_route(&engine, entry, required(&params, "uri")?)?,
            ),
            "tools/call" | "prompts/get" => {
                let kind = if method == "tools/call" {
                    CatalogKind::Tools
                } else {
                    CatalogKind::Prompts
                };
                let (item, route) = named_item(
                    &engine,
                    entry,
                    kind,
                    required(&params, "name")?,
                    expected.route.as_ref(),
                )
                .await?
                .context("此工具或提示不在入口清单中")?;
                if let Some(hash) = &expected.schema_hash {
                    ensure!(
                        crate::invocation::schema_hash(&item)? == *hash,
                        "工具 Schema 已变化，请重新核对"
                    );
                }
                ("name", route)
            }
            _ => bail!("聚合入口未实现此 MCP 能力"),
        };
        if let Some(pin) = &expected.revision {
            ensure!(
                matches!(pin, RevisionPin::Exact(value) if *value == route.revision),
                "发现期间服务已更新，请重新调用"
            );
        }
        if let Some(prior) = &expected.route {
            ensure!(
                prior.original_name == route.original_name,
                "工具路由已变化，请重新发现清单"
            );
        }
        let mut forwarded = request;
        forwarded["params"][key] = json!(route.original_name);
        if let Some(meta) = forwarded
            .pointer_mut("/params/_meta")
            .and_then(Value::as_object_mut)
        {
            meta.remove("mcplink");
        }
        current(&engine, entry)?;
        let mut result = Box::pin(engine.pool.request_at_revision(
            engine.clone(),
            &route.connection_id,
            forwarded,
            Some(route.revision),
        ))
        .await?;
        rewrite_result(&mut result, &route.namespace);
        Ok(result)
    })
}
