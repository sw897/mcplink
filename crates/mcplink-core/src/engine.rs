use crate::{
    config,
    mcp::McpPool,
    model::*,
    store::Store,
    vault::{Vault, private_dir, private_write},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;

pub struct Engine {
    pub data: PathBuf,
    pub store: Arc<Store>,
    pub vault: Vault,
    pub pool: McpPool,
    pub mutation: Mutex<()>,
    pub paused: AtomicBool,
}

impl Engine {
    pub fn open(data: PathBuf) -> Result<Arc<Self>> {
        let store = Arc::new(Store::open(&data)?);
        let vault = Vault::new(&data)?;
        let engine = Arc::new(Self {
            data,
            store,
            vault,
            pool: McpPool::default(),
            mutation: Mutex::new(()),
            paused: AtomicBool::new(false),
        });
        engine.reconcile()?;
        Ok(engine)
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let mut sources = config::default_sources()?;
        sources.extend(self.store.sources()?);
        Ok(sources)
    }

    pub fn source(&self, id: &str) -> Result<Source> {
        self.sources()?
            .into_iter()
            .find(|source| source.id == id)
            .context("配置来源不存在")
    }

    pub fn definition(&self, connection: &Connection) -> Result<Definition> {
        let raw = self
            .vault
            .get(&connection.definition_ref)?
            .context("连接数据未能从系统凭据库取得")?;
        serde_json::from_str(&raw).context("连接数据无法读取")
    }

    pub(crate) fn save_definition(
        &self,
        connection: &mut Connection,
        definition: &Definition,
    ) -> Result<()> {
        let old = connection.definition_ref.clone();
        let reset_oauth = if old.is_empty() {
            false
        } else {
            let previous = self.definition(connection)?;
            previous.url != definition.url
                || previous.client_id != definition.client_id
                || previous.client_secret != definition.client_secret
                || previous.oauth_scopes != definition.oauth_scopes
                || previous.oauth_callback_port != definition.oauth_callback_port
        };
        if reset_oauth && definition.oauth {
            connection.status = "needs_auth".into();
        }
        let reference = format!("definition/{}", uuid::Uuid::new_v4());
        self.vault
            .set(&reference, &serde_json::to_string(definition)?)?;
        connection.definition_ref = reference;
        connection.revision += 1;
        connection.transport = definition.transport().into();
        connection.endpoint = definition.summary();
        self.store.save_connection(connection)?;
        if reset_oauth {
            self.vault.delete(&format!("oauth/{}", connection.id))?;
        }
        if !old.is_empty() {
            self.vault.delete(&old)?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Value> {
        let connections = self.store.connections()?;
        let bindings = self.store.bindings()?;
        let mut scopes = Vec::new();
        let mut detected = Vec::new();
        let mut rows: Vec<Value> = connections.iter().map(Connection::public_value).collect();
        for source in self.sources()? {
            let result = config::scan(&source);
            let error = result.as_ref().err().map(ToString::to_string);
            scopes.push(json!({"id":source.id,"agent":source.agent,"label":source.label,"path":source.path,"exists":source.path.exists(),"error":error,"readOnly":source.read_only}));
            if let Ok(entries) = result {
                for entry in entries {
                    let binding = bindings
                        .iter()
                        .find(|b| b.source_id == source.id && b.name == entry.name);
                    let connection_id = binding.map(|binding| binding.connection_id.clone());
                    let id = format!(
                        "detected:{}",
                        digest(format!("{}:{}", source.id, entry.name).as_bytes())
                    );
                    let invalid = entry.error.is_some();
                    if connection_id.is_none() {
                        rows.push(json!({"id":id,"name":entry.name,"title":entry.name,"transport":entry.definition.transport(),"endpoint":entry.definition.summary(),"accountLabel":if entry.definition.has_credentials(){"由客户端维护"}else{"本地工具 / 原生服务"},"authKind":if entry.definition.has_credentials(){"native"}else{"none"},"active":true,"status":if invalid{"error"}else{"configured"},"lastError":entry.error,"detected":true,"sourceId":source.id}));
                    }
                    let changed =
                        binding.is_some_and(|binding| binding.fingerprint != entry.fingerprint);
                    let needs_update = binding.is_some_and(|b| {
                        b.mode == "direct"
                            && connections
                                .iter()
                                .any(|c| c.id == b.connection_id && c.revision != b.revision)
                    });
                    detected.push(json!({"id":id,"connectionId":connection_id,"sourceId":source.id,"agent":source.agent,"name":entry.name,"enabled":entry.definition.enabled,"transport":entry.definition.transport(),"changed":changed,"needsUpdate":needs_update,"error":entry.error,"mode":binding.map(|b|b.mode.as_str()).unwrap_or("direct")}));
                }
            }
        }
        for binding in &bindings {
            if !detected
                .iter()
                .any(|row| row["sourceId"] == binding.source_id && row["name"] == binding.name)
                && let Some(source) = scopes
                    .iter()
                    .find(|source| source["id"] == binding.source_id && source["error"].is_null())
            {
                detected.push(json!({"id":format!("missing:{}:{}",binding.source_id,binding.name),"connectionId":binding.connection_id,"sourceId":binding.source_id,"agent":source["agent"],"name":binding.name,"enabled":false,"changed":true,"missing":true,"mode":binding.mode}));
            }
        }
        let history = self
            .store
            .history()?
            .into_iter()
            .take(25)
            .map(|mut value| {
                if let Some(m) = value.as_object_mut() {
                    m.remove("credentialRef");
                    m.remove("backup");
                }
                value
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"connections":rows,"bindings":detected,"sources":scopes,"history":history,"gateway":{"paused":self.paused.load(Ordering::SeqCst),"sessions":self.pool.count()},"platform":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION")}),
        )
    }

    fn import(&self, source_id: &str, name: &str) -> Result<Value> {
        let source = self.source(source_id)?;
        let entry = config::scan(&source)?
            .into_iter()
            .find(|entry| entry.name == name)
            .context("来源中的服务已变化，请刷新")?;
        if let Some(binding) = self
            .store
            .bindings()?
            .iter()
            .find(|b| b.source_id == source_id && b.name == name)
        {
            return Ok(self
                .store
                .connection(&binding.connection_id)?
                .public_value());
        }
        if let Some(error) = entry.error {
            bail!("该服务配置无效：{error}");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut connection = Connection {
            id: id.clone(),
            name: name.into(),
            title: name.into(),
            transport: entry.definition.transport().into(),
            endpoint: entry.definition.summary(),
            account_label: if entry.definition.has_credentials() {
                "工作账号（用户标签）".into()
            } else {
                "原生服务".into()
            },
            auth_kind: if entry.definition.oauth {
                "oauth"
            } else if !entry.definition.headers.is_empty() {
                "token"
            } else {
                "none"
            }
            .into(),
            definition_ref: String::new(),
            revision: 0,
            active: true,
            status: "configured".into(),
            checked_at: None,
            tool_count: None,
            last_error: None,
        };
        self.save_definition(&mut connection, &entry.definition)?;
        self.store.bind(&Binding {
            connection_id: id,
            source_id: source_id.into(),
            name: name.into(),
            mode: "direct".into(),
            fingerprint: entry.fingerprint,
            revision: connection.revision,
        })?;
        Ok(connection.public_value())
    }

    fn save(&self, p: &Value) -> Result<Value> {
        let name = required(p, "name")?;
        config::safe_name(name)?;
        let mut definition: Definition =
            serde_json::from_value(p.get("definition").cloned().context("缺少服务配置")?)
                .context("服务配置格式有误")?;
        definition.enabled = p
            .pointer("/definition/enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        config::validate(&definition)?;
        let existing = p
            .get("id")
            .and_then(Value::as_str)
            .map(|id| self.store.connection(id))
            .transpose()?;
        if let Some(old) = &existing {
            let old_definition = self.definition(old)?;
            if definition.client_secret.as_deref() == Some("••••••••") {
                definition.client_secret = Some(
                    old_definition
                        .client_secret
                        .clone()
                        .context("需要重新填写 OAuth 客户端机密")?,
                );
            }
            for (key, value) in &mut definition.headers {
                if value == "••••••••" {
                    *value = old_definition
                        .headers
                        .get(key)
                        .cloned()
                        .context("需要重新填写该认证字段")?;
                }
            }
            for (key, value) in &mut definition.env {
                if value == "••••••••" {
                    *value = old_definition
                        .env
                        .get(key)
                        .cloned()
                        .context("需要重新填写该环境变量")?;
                }
            }
            for (index, value) in definition.args.iter_mut().enumerate() {
                if value == "••••••••" {
                    *value = old_definition
                        .args
                        .get(index)
                        .cloned()
                        .context("请重新填写启动参数")?;
                }
            }
            if definition
                .url
                .as_ref()
                .is_some_and(|value| value.contains("••••"))
            {
                definition.url = old_definition.url;
            }
            definition.original = old_definition.original;
            definition.original_agent = old_definition.original_agent;
        }
        let mut connection = existing.unwrap_or_else(|| Connection {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            title: name.into(),
            transport: String::new(),
            endpoint: String::new(),
            account_label: String::new(),
            auth_kind: "none".into(),
            definition_ref: String::new(),
            revision: 0,
            active: true,
            status: "configured".into(),
            checked_at: None,
            tool_count: None,
            last_error: None,
        });
        if self
            .store
            .bindings()?
            .iter()
            .any(|b| b.connection_id == connection.id)
            && connection.name != name
        {
            bail!("已有 Agent 绑定时请保留接入名称，或先移除相关绑定");
        }
        connection.name = name.into();
        connection.title = p
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .into();
        connection.account_label = p
            .get("accountLabel")
            .and_then(Value::as_str)
            .unwrap_or("用户标签")
            .into();
        connection.auth_kind = if definition.oauth {
            "oauth"
        } else if !definition.headers.is_empty() {
            "token"
        } else {
            "none"
        }
        .into();
        connection.active = true;
        connection.status = "configured".into();
        connection.checked_at = None;
        connection.tool_count = None;
        connection.last_error = None;
        self.save_definition(&mut connection, &definition)?;
        Ok(connection.public_value())
    }

    fn preview(&self, p: &Value) -> Result<Value> {
        let ids: Vec<String> =
            if let Some(values) = p.get("connectionIds").and_then(Value::as_array) {
                values
                    .iter()
                    .map(|value| value.as_str().map(str::to_owned).context("服务标识无效"))
                    .collect::<Result<_>>()?
            } else {
                vec![required(p, "connectionId")?.to_owned()]
            };
        let mode = p.get("mode").and_then(Value::as_str).unwrap_or("direct");
        let action = p.get("action").and_then(Value::as_str).unwrap_or("upsert");
        if !matches!(mode, "direct" | "shared")
            || !matches!(action, "upsert" | "remove" | "disable" | "enable")
        {
            bail!("配置操作或接入方式无效");
        }
        let targets = p
            .get("targetSourceIds")
            .and_then(Value::as_array)
            .context("请选择目标 Agent")?;
        if ids.is_empty() || targets.is_empty() {
            bail!("请选择服务与目标 Agent");
        }
        let bindings = self.store.bindings()?;
        let mut changes = Vec::new();
        let mut virtual_files = std::collections::HashMap::<PathBuf, String>::new();
        let mut touched = std::collections::HashSet::new();
        let mut connections = Vec::new();
        for id in ids {
            let connection = self.store.connection(&id)?;
            let definition = self.definition(&connection)?;
            if action == "upsert"
                && mode == "shared"
                && (!connection.active || connection.status != "ready")
            {
                bail!("请先检查共享连接并完成账号认证");
            }
            for target in targets {
                let source = self.source(target.as_str().context("目标配置无效")?)?;
                if source.read_only {
                    bail!(
                        "{} 来源只读，请在原客户端或插件中维护配置",
                        source.agent.label()
                    );
                }
                if fs::symlink_metadata(&source.path)
                    .is_ok_and(|meta| meta.file_type().is_symlink())
                {
                    bail!("目标配置为符号链接，请添加真实配置文件作为来源");
                }
                let text = match virtual_files.get(&source.path) {
                    Some(text) => text.clone(),
                    None => config::read(&source)?.0,
                };
                let document = config::parse(&source, &text)?;
                let binding = bindings
                    .iter()
                    .find(|b| b.connection_id == connection.id && b.source_id == source.id);
                let name = binding
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| connection.name.clone());
                if !touched.insert((source.path.clone(), config::name_key(&name))) {
                    bail!("批量操作中存在重复目标或接入名称冲突");
                }
                let before = document
                    .get("mcp_servers")
                    .and_then(|servers| servers.get(&name))
                    .cloned();
                if let Some(binding) = binding {
                    if config::fingerprint(before.as_ref()) != binding.fingerprint
                        && !(action == "remove" && before.is_none())
                    {
                        bail!(
                            "{} 的条目有外部修改，请先采用来源更新或核对配置",
                            source.agent.label()
                        );
                    }
                } else if before.is_some() {
                    bail!("{} 已有同名服务，请先关联已有连接", source.agent.label());
                }
                if document
                    .get("mcp_servers")
                    .and_then(Value::as_object)
                    .is_some_and(|servers| {
                        servers.keys().any(|key| {
                            key != &name && config::name_key(key) == config::name_key(&name)
                        })
                    })
                {
                    bail!("目标存在工具名称冲突，请调整接入名称");
                }
                let actual_mode = if action == "upsert" {
                    mode
                } else {
                    binding.map(|b| b.mode.as_str()).unwrap_or(mode)
                };
                let after = if action == "remove" {
                    None
                } else {
                    let mut value = if matches!(action, "disable" | "enable") {
                        before.clone().context("目标尚未接入此服务")?
                    } else if actual_mode == "shared" {
                        self.bridge_config(
                            &connection.id,
                            p.get("_bridgeExecutable").and_then(Value::as_str),
                        )?
                    } else {
                        config::native(&definition, &source.agent)?
                    };
                    if action == "disable" {
                        value["enabled"] = json!(false);
                    }
                    if action == "enable" {
                        value["enabled"] = json!(true);
                    }
                    Some(value)
                };
                let rendered = config::render(&source, &text, &name, after.as_ref())?;
                virtual_files.insert(source.path.clone(), rendered);
                changes.push(Change {
                    id: uuid::Uuid::new_v4().to_string(),
                    connection_id: connection.id.clone(),
                    source_id: source.id,
                    name,
                    mode: actual_mode.into(),
                    action: action.into(),
                    expected_hash: digest(text.as_bytes()),
                    revision: connection.revision,
                    previous_binding: binding.cloned(),
                    restore_unmanaged: false,
                    before,
                    after,
                });
            }
            connections.push(connection);
        }
        let plan = ChangePlan {
            id: uuid::Uuid::new_v4().to_string(),
            changes,
            created_at: now(),
        };
        self.vault
            .set(&format!("plan/{}", plan.id), &serde_json::to_string(&plan)?)?;
        let summary=plan.changes.iter().map(|change|{
            let source=self.source(&change.source_id)?;
            Ok(json!({"id":change.id,"sourceId":source.id,"agent":source.agent,"sourceLabel":source.label,"path":source.path,"name":change.name,"mode":change.mode,"action":change.action,"before":change.before.as_ref().and_then(|v|config::normalize(v,source.agent.clone()).ok()).map(|d|d.public_value()),"after":change.after.as_ref().and_then(|v|config::normalize(v,source.agent.clone()).ok()).map(|d|d.public_value()),"reload":"配置写入后，请在对应 Agent 重新加载 MCP 或重新开启会话"}))
        }).collect::<Result<Vec<_>>>()?;
        let mut display = connections[0].public_value();
        if connections.len() > 1 {
            display["title"] = json!(format!("{} 项服务", connections.len()));
        }
        Ok(
            json!({"planId":plan.id,"connection":display,"changes":summary,"expiresAt":plan.created_at+600}),
        )
    }

    fn bridge_config(&self, id: &str, client_executable: Option<&str>) -> Result<Value> {
        let executable = client_executable
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(std::env::current_exe)
            .context("无法定位 mcplink 程序")?;
        if !executable.is_absolute() || !executable.is_file() {
            bail!("无法定位共享入口程序，请从已安装应用重新预览");
        }
        let mut env = json!({"MCPLINK_DATA_DIR":self.data});
        for key in ["MCPLINK_HOME", "MCPLINK_VAULT"] {
            if let Ok(value) = std::env::var(key) {
                env[key] = json!(value);
            }
        }
        Ok(
            json!({"command":executable,"args":["bridge","--connection",id],"env":env,"enabled":true}),
        )
    }

    fn apply_change(&self, change: &Change, plan_id: &str) -> Result<()> {
        if change.action != "restore"
            && change.revision != 0
            && self.store.connection(&change.connection_id)?.revision != change.revision
        {
            bail!("连接已更新，请重新预览");
        }
        let source = self.source(&change.source_id)?;
        let (text, _) = config::read(&source)?;
        if digest(text.as_bytes()) != change.expected_hash {
            bail!("目标文件已变化，请重新预览");
        }
        let parent = source.path.parent().context("配置位置无效")?;
        private_dir(parent)?;
        let lockfile = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join(".mcplink.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lockfile)
            .context("另一个配置操作正在进行，请稍后重试")?;
        let (current, _) = config::read(&source)?;
        if digest(current.as_bytes()) != change.expected_hash {
            bail!("目标文件已被外部修改，请重新预览");
        }
        let rendered = config::render(&source, &current, &change.name, change.after.as_ref())?;
        let reference = format!("change/{}", change.id);
        self.vault
            .set(&reference, &serde_json::to_string(change)?)?;
        let backup = self
            .data
            .join("backups")
            .join(format!("{}.config", change.id));
        self.store
            .record_operation(change, plan_id, &reference, &backup.to_string_lossy())?;
        let result = (|| -> Result<()> {
            private_write(&backup, current.as_bytes())?;
            if std::env::var("MCPLINK_FAILPOINT").ok().as_deref() == Some("before-write")
                && std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture")
            {
                bail!("测试注入的写入前中断");
            }
            if digest(config::read(&source)?.0.as_bytes()) != change.expected_hash {
                bail!("目标文件已被外部修改，请重新预览");
            }
            private_write(&source.path, rendered.as_bytes())?;
            if std::env::var("MCPLINK_FAILPOINT").ok().as_deref() == Some("after-write")
                && std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture")
            {
                std::process::exit(93);
            }
            self.update_binding(change)?;
            self.store.finish_operation(&change.id, "applied", None)?;
            Ok(())
        })();
        if let Err(error) = &result {
            self.store
                .finish_operation(&change.id, "failed", Some(&error.to_string()))?;
        }
        result
    }

    fn update_binding(&self, change: &Change) -> Result<()> {
        if change.after.is_some() && !change.restore_unmanaged {
            self.store.bind(&Binding {
                connection_id: change.connection_id.clone(),
                source_id: change.source_id.clone(),
                name: change.name.clone(),
                mode: change.mode.clone(),
                fingerprint: config::fingerprint(change.after.as_ref()),
                revision: if change.revision == 0 {
                    self.store.connection(&change.connection_id)?.revision
                } else {
                    change.revision
                },
            })?;
        } else {
            self.store.unbind(&change.source_id, &change.name)?;
        }
        Ok(())
    }

    fn apply(&self, id: &str) -> Result<Value> {
        let plan: ChangePlan = serde_json::from_str(
            &self
                .vault
                .get(&format!("plan/{id}"))?
                .context("预览已失效，请重新预览")?,
        )?;
        if now() > plan.created_at + 600 {
            bail!("预览已过期，请重新预览");
        }
        let history = self.store.history()?;
        let mut results = Vec::new();
        for change in &plan.changes {
            if let Some(done) = history.iter().find(|value| value["id"] == change.id) {
                results.push(json!({"sourceId":change.source_id,"status":done["status"],"operationId":change.id,"message":"该目标已有执行记录，请核对历史结果"}));
                continue;
            }
            match self.apply_change(change,id) {
                Ok(())=>results.push(json!({"sourceId":change.source_id,"status":"applied","operationId":change.id,"message":"配置已写入，等待 Agent 重新加载"})),
                Err(error)=>results.push(json!({"sourceId":change.source_id,"status":"failed","operationId":change.id,"message":error.to_string()})),
            }
        }
        Ok(json!({"results":results}))
    }

    fn restore(&self, id: &str) -> Result<Value> {
        let operation = self.store.operation(id)?;
        if operation["status"] == "restored" {
            return Ok(json!({"status":"restored"}));
        }
        let reference = operation["credentialRef"]
            .as_str()
            .context("变更记录无效")?;
        let original: Change = serde_json::from_str(
            &self
                .vault
                .get(reference)?
                .context("恢复所需的连接数据不可用")?,
        )?;
        let source = self.source(&original.source_id)?;
        let current = config::entry(&source, &original.name)?;
        if current != original.after {
            bail!("该条目有后续修改，请核对差异后处理，其他配置保持当前版本");
        }
        let text = config::read(&source)?.0;
        let reversed = Change {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id: original.connection_id,
            source_id: original.source_id,
            name: original.name.clone(),
            mode: original
                .previous_binding
                .as_ref()
                .map(|b| b.mode.clone())
                .unwrap_or_else(|| "direct".into()),
            revision: original
                .previous_binding
                .as_ref()
                .map(|b| b.revision)
                .unwrap_or(0),
            previous_binding: self
                .store
                .bindings()?
                .into_iter()
                .find(|b| b.source_id == source.id && b.name == original.name),
            restore_unmanaged: original.previous_binding.is_none(),
            action: "restore".into(),
            expected_hash: digest(text.as_bytes()),
            before: current,
            after: original.before,
        };
        self.apply_change(&reversed, &format!("restore/{id}"))?;
        self.store.finish_operation(id, "restored", None)?;
        Ok(json!({"status":"restored","operationId":reversed.id}))
    }

    fn reconcile(&self) -> Result<()> {
        for operation in self.store.history()? {
            if operation["status"] != "pending" {
                continue;
            }
            let id = operation["id"].as_str().context("变更记录缺少标识")?;
            let reference = operation["credentialRef"]
                .as_str()
                .context("变更记录无效")?;
            let Some(raw) = self.vault.get(reference)? else {
                self.store.finish_operation(
                    id,
                    "conflict",
                    Some("变更数据不可用，需要核对配置"),
                )?;
                continue;
            };
            let change: Change = serde_json::from_str(&raw)?;
            let source = self.source(&change.source_id)?;
            match config::entry(&source, &change.name) {
                Ok(current) if current == change.after => {
                    self.update_binding(&change)?;
                    self.store.finish_operation(id, "applied", None)?;
                }
                Ok(current) if current == change.before => {
                    self.store
                        .finish_operation(id, "failed", Some("中断前尚未写入该条目"))?
                }
                _ => self.store.finish_operation(
                    id,
                    "conflict",
                    Some("中断后配置有变化，请核对当前条目"),
                )?,
            }
        }
        Ok(())
    }

    pub async fn rpc(self: &Arc<Self>, method: &str, p: Value) -> Result<Value> {
        match method {
            "snapshot" => self.snapshot(),
            "workspace.identity" => {
                let _guard = self.mutation.lock().await;
                let path = self.data.join("dataset-id");
                let id = match fs::read_to_string(&path) {
                    Ok(id) => {
                        uuid::Uuid::parse_str(&id).context("数据目录身份无效，请核对本地数据")?;
                        id
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let id = uuid::Uuid::new_v4().to_string();
                        private_write(&path, id.as_bytes())?;
                        id
                    }
                    Err(_) => bail!("无法读取数据目录身份"),
                };
                Ok(json!({"id":id}))
            }
            "sources.add" => {
                let _guard = self.mutation.lock().await;
                let agent: Agent =
                    serde_json::from_value(p.get("agent").cloned().context("请选择 Agent")?)?;
                let path = PathBuf::from(required(&p, "path")?);
                let path = path.canonicalize().unwrap_or(path);
                if let Some(existing) = self
                    .sources()?
                    .into_iter()
                    .find(|s| s.path.canonicalize().unwrap_or(s.path.clone()) == path)
                {
                    return Ok(serde_json::to_value(existing)?);
                }
                if !path.is_absolute() {
                    bail!("请选择完整的配置文件路径");
                }
                let id = format!(
                    "source:{}",
                    digest(format!("{}:{}", agent.label(), path.display()).as_bytes())
                );
                let source = Source {
                    id,
                    read_only: agent == Agent::Claude,
                    agent,
                    label: p
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("自定义配置")
                        .into(),
                    path,
                    project: None,
                    plugin_root: None,
                    discovery_error: None,
                };
                config::read(&source)?;
                self.store.save_source(&source)?;
                Ok(serde_json::to_value(source)?)
            }
            "connections.import" => {
                let _guard = self.mutation.lock().await;
                self.import(required(&p, "sourceId")?, required(&p, "name")?)
            }
            "connections.adopt_source" => {
                let _guard = self.mutation.lock().await;
                let source_id = required(&p, "sourceId")?;
                let name = required(&p, "name")?;
                let mut binding = self
                    .store
                    .bindings()?
                    .into_iter()
                    .find(|b| b.source_id == source_id && b.name == name)
                    .context("关联不存在")?;
                if binding.mode != "direct" {
                    bail!("共享入口不能作为上游配置，请编辑服务连接");
                }
                let entry = config::scan(&self.source(source_id)?)?
                    .into_iter()
                    .find(|e| e.name == name)
                    .context("来源中已移除此服务")?;
                if let Some(error) = entry.error {
                    bail!("来源配置无效：{error}");
                }
                let mut connection = self.store.connection(&binding.connection_id)?;
                connection.status = "configured".into();
                connection.checked_at = None;
                connection.tool_count = None;
                connection.last_error = None;
                self.save_definition(&mut connection, &entry.definition)?;
                binding.fingerprint = entry.fingerprint;
                binding.revision = connection.revision;
                self.store.bind(&binding)?;
                self.pool.invalidate(&connection.id).await;
                Ok(connection.public_value())
            }
            "connections.save" => {
                let _guard = self.mutation.lock().await;
                let result = self.save(&p)?;
                self.pool
                    .invalidate(result["id"].as_str().unwrap_or(""))
                    .await;
                Ok(result)
            }
            "connections.detail" => {
                let connection = self.store.connection(required(&p, "id")?)?;
                Ok(
                    json!({"connection":connection.public_value(),"definition":self.definition(&connection)?.public_value()}),
                )
            }
            "connections.delete" => {
                let _guard = self.mutation.lock().await;
                let id = required(&p, "id")?;
                if self.store.bindings()?.iter().any(|b| b.connection_id == id) {
                    bail!("请先移除各 Agent 的配置绑定，再删除连接");
                }
                let connection = self.store.connection(id)?;
                self.store.delete_connection(id)?;
                self.vault.delete(&connection.definition_ref)?;
                self.vault.delete(&format!("oauth/{id}"))?;
                self.pool.invalidate(id).await;
                Ok(json!({"deleted":true}))
            }
            "changes.preview" => {
                let _guard = self.mutation.lock().await;
                self.preview(&p)
            }
            "changes.apply" => {
                let _guard = self.mutation.lock().await;
                self.apply(required(&p, "planId")?)
            }
            "changes.restore" => {
                let _guard = self.mutation.lock().await;
                self.restore(required(&p, "operationId")?)
            }
            "connections.probe" | "connections.tools" => self.probe(required(&p, "id")?).await,
            "credentials.update" => {
                let _guard = self.mutation.lock().await;
                let mut connection = self.store.connection(required(&p, "id")?)?;
                let mut definition = self.definition(&connection)?;
                let header = p
                    .get("header")
                    .and_then(Value::as_str)
                    .unwrap_or("Authorization");
                definition
                    .headers
                    .retain(|key, _| !key.eq_ignore_ascii_case(header));
                definition.headers.insert(
                    p.get("header")
                        .and_then(Value::as_str)
                        .unwrap_or("Authorization")
                        .into(),
                    required(&p, "value")?.into(),
                );
                definition.oauth = false;
                if let Some(original) = definition.original.as_object_mut() {
                    for key in [
                        "headers",
                        "http_headers",
                        "env_http_headers",
                        "bearer_token_env_var",
                        "oauth",
                    ] {
                        original.remove(key);
                    }
                }
                self.pool.invalidate(&connection.id).await;
                let refresh = self.pool.refresh_lock(&connection.id);
                let _refresh = refresh.lock().await;
                self.vault.delete(&format!("oauth/{}", connection.id))?;
                connection.auth_kind = "token".into();
                connection.active = true;
                connection.status = "configured".into();
                connection.checked_at = None;
                connection.tool_count = None;
                connection.last_error = None;
                self.save_definition(&mut connection, &definition)?;
                drop(_refresh);
                self.pool.invalidate(&connection.id).await;
                Ok(connection.public_value())
            }
            "credentials.disconnect" => {
                let _guard = self.mutation.lock().await;
                let mut connection = self.store.connection(required(&p, "id")?)?;
                self.pool.invalidate(&connection.id).await;
                let refresh = self.pool.refresh_lock(&connection.id);
                let _refresh = refresh.lock().await;
                let mut definition = self.definition(&connection)?;
                definition.headers.clear();
                if let Some(map) = definition.original.as_object_mut() {
                    for key in [
                        "headers",
                        "http_headers",
                        "env_http_headers",
                        "bearer_token_env_var",
                        "oauth",
                    ] {
                        map.remove(key);
                    }
                }
                connection.active = false;
                connection.status = "disconnected".into();
                connection.checked_at = None;
                connection.tool_count = None;
                self.save_definition(&mut connection, &definition)?;
                self.vault.delete(&format!("oauth/{}", connection.id))?;
                Ok(connection.public_value())
            }
            "oauth.start" => {
                let id = required(&p, "id")?;
                let result = crate::mcp::start_oauth(self.clone(), id).await;
                if result.is_err() {
                    let _guard = self.mutation.lock().await;
                    let mut c = self.store.connection(id)?;
                    if c.status == "authorizing" {
                        c.status = "needs_auth".into();
                        c.last_error = Some("未能开始 OAuth 授权，请检查服务认证设置".into());
                        self.store.save_connection(&c)?;
                    }
                }
                result
            }
            "oauth.configure" => crate::mcp::configure_oauth(self.clone(), &p).await,
            "gateway.pause" => {
                self.paused.store(true, Ordering::SeqCst);
                self.pool.clear().await;
                Ok(json!({"paused":true}))
            }
            "gateway.resume" => {
                self.paused.store(false, Ordering::SeqCst);
                Ok(json!({"paused":false}))
            }
            "tools.describe" | "tools.call" => {
                if self.paused.load(Ordering::SeqCst) {
                    bail!("共享后台已暂停，请在 mcplink 恢复");
                }
                let id = required(&p, "id")?;
                let name = required(&p, "tool")?;
                if method == "tools.describe" {
                    crate::invocation::describe(self.clone(), id, name).await
                } else {
                    crate::invocation::call(
                        self.clone(),
                        id,
                        name,
                        p.get("arguments").cloned().context("缺少工具参数对象")?,
                        p.get("schemaHash").and_then(Value::as_str),
                    )
                    .await
                }
            }
            "mcp.describe" => {
                if self.paused.load(Ordering::SeqCst) {
                    bail!("共享后台已暂停，请在 mcplink 恢复");
                }
                let peer = self.pool.peer(self.clone(), required(&p, "id")?).await?;
                let mut info =
                    serde_json::to_value(peer.peer_info().context("上游未返回服务信息")?)?;
                let definition = self.definition(&self.store.connection(required(&p, "id")?)?)?;
                if let Some(capabilities) =
                    info.get_mut("capabilities").and_then(Value::as_object_mut)
                {
                    if definition.resources_disabled {
                        capabilities.remove("resources");
                    }
                    if definition.prompts_disabled {
                        capabilities.remove("prompts");
                    }
                }
                Ok(info)
            }
            "mcp.request" => {
                if self.paused.load(Ordering::SeqCst) {
                    bail!("共享后台已暂停，请在 mcplink 恢复");
                }
                self.pool
                    .request(
                        self.clone(),
                        required(&p, "id")?,
                        p.get("request").cloned().context("MCP 请求缺失")?,
                    )
                    .await
            }
            "browser.open" => {
                let url = url::Url::parse(required(&p, "url")?).context("浏览器地址无效")?;
                if url.scheme() != "https" {
                    bail!("浏览器入口需要 HTTPS 地址");
                }
                opener::open(url.as_str()).context("无法打开系统浏览器")?;
                Ok(json!({"opened":true}))
            }
            _ => bail!("不支持的本地管理操作"),
        }
    }

    async fn probe(self: &Arc<Self>, id: &str) -> Result<Value> {
        let expected_revision = self.store.connection(id)?.revision;
        let temporary = McpPool::default();
        let shared = self
            .store
            .bindings()?
            .iter()
            .any(|binding| binding.connection_id == id && binding.mode == "shared");
        let pool = if shared { &self.pool } else { &temporary };
        let result = crate::invocation::discover_with_pool(self.clone(), id, pool)
            .await
            .map(|(_, tools)| json!({"tools":tools}));
        temporary.clear().await;
        let _mutation = self.mutation.lock().await;
        let mut connection = self.store.connection(id)?;
        if connection.revision != expected_revision {
            bail!("检查期间连接已更新，请重新检查");
        }
        connection.checked_at = Some(now());
        match result {
            Ok(value) => {
                if let Some(error) = value.get("rpcError") {
                    connection.status = "error".into();
                    connection.last_error = Some("上游工具发现返回错误".into());
                    self.store.save_connection(&connection)?;
                    return Ok(json!({"connection":connection.public_value(),"error":error}));
                }
                let tools = value
                    .get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                connection.status = "ready".into();
                connection.tool_count = Some(tools.len());
                connection.last_error = None;
                self.store.save_connection(&connection)?;
                Ok(
                    json!({"connection":connection.public_value(),"tools":tools,"schemaHash":digest(&serde_json::to_vec(&value)?) }),
                )
            }
            Err(error) => {
                connection.status = if !connection.active {
                    "disconnected"
                } else {
                    "error"
                }
                .into();
                if connection.active
                    && (error.to_string().contains("认证") || error.to_string().contains("授权"))
                {
                    connection.status = "needs_auth".into();
                }
                connection.tool_count = None;
                connection.last_error = Some(error.to_string());
                self.store.save_connection(&connection)?;
                Ok(
                    json!({"connection":connection.public_value(),"error":error.to_string(),"tools":[]}),
                )
            }
        }
    }
}

pub fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .with_context(|| format!("缺少字段 {key}"))
}
