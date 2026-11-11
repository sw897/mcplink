use crate::{
    config,
    mcp::McpPool,
    model::*,
    store::{ConnectionDeletion, Store},
    vault::{Vault, private_dir, private_write},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;

pub struct Engine {
    pub data: PathBuf,
    draft_key: uuid::Uuid,
    pub store: Arc<Store>,
    pub vault: Vault,
    pub pool: McpPool,
    pub(crate) catalog: crate::gateway::CatalogCache,
    pub mutation: Mutex<()>,
    pub paused: AtomicBool,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}

impl Engine {
    pub async fn dataset_id(&self) -> Result<String> {
        let _guard = self.mutation.lock().await;
        let path = self.data.join("dataset-id");
        match fs::read_to_string(&path) {
            Ok(id) => {
                uuid::Uuid::parse_str(&id).context("数据目录身份无效，请核对本地数据")?;
                Ok(id)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let id = uuid::Uuid::new_v4().to_string();
                private_write(&path, id.as_bytes())?;
                Ok(id)
            }
            Err(_) => bail!("无法读取数据目录身份"),
        }
    }
    pub fn open(data: PathBuf) -> Result<Arc<Self>> {
        let store = Arc::new(Store::open(&data)?);
        let paused = store.gateway_paused()?;
        let vault = Vault::new(&data)?;
        let engine = Arc::new(Self {
            data,
            draft_key: uuid::Uuid::new_v4(),
            store,
            vault,
            pool: McpPool::default(),
            catalog: crate::gateway::CatalogCache::default(),
            mutation: Mutex::new(()),
            paused: AtomicBool::new(paused),
            shutdown: tokio::sync::watch::channel(false).0,
        });
        engine.reconcile()?;
        for (id, _) in engine.store.pending_credential_cleanup()? {
            if crate::lifecycle::cleanup_credentials(&engine, &id).is_err() {
                eprintln!("mcplink: 连接 {id} 的旧凭据清理待重试");
            }
        }
        for (id, definition_ref) in engine.store.pending_credential_deletions()? {
            if engine
                .finish_credential_deletion(&id, &definition_ref)
                .is_err()
            {
                eprintln!("mcplink: 连接 {id} 的凭据清理待重试");
            }
        }
        Ok(engine)
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let mut sources = config::default_sources()?;
        for stored in self.store.sources()? {
            if let Some(position) = sources.iter().position(|s| {
                s.id == stored.id
                    || (matches!(stored.agent, Agent::Workbuddy | Agent::Qwenwork)
                        && s.agent == stored.agent
                        && s.path
                            == config::canonical_path(&stored.path).unwrap_or(stored.path.clone())
                        && s.project == stored.project)
            }) {
                if (stored.agent == Agent::Opencode && stored.path == sources[position].path)
                    || matches!(stored.agent, Agent::Workbuddy | Agent::Qwenwork)
                {
                    sources[position] = stored;
                }
            } else {
                sources.push(stored);
            }
        }
        if let Some(error) = crate::agent_config::qwen_storage_error()? {
            for source in sources
                .iter_mut()
                .filter(|source| source.agent == Agent::Qwenwork)
            {
                source.read_only = true;
                source.discovery_error = Some(error.clone());
            }
        }
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

    pub fn snapshot(&self) -> Result<Value> {
        let connections = self.store.connections()?;
        let bindings = self.store.bindings()?;
        let mut scopes = Vec::new();
        let mut detected = Vec::new();
        let mut rows: Vec<Value> = connections.iter().map(Connection::public_value).collect();
        let sources = self.sources()?;
        let agents = crate::discovery::inventory(&sources, &self.store)?;
        for source in sources {
            let result = config::scan(&source);
            let error = result.as_ref().err().map(ToString::to_string);
            scopes.push(json!({"id":source.id,"agent":source.agent,"label":source.label,"path":source.path,"project":source.project,"scope":source.scope,"exists":source.path.exists(),"error":error,"readOnly":source.read_only}));
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
                    detected.push(json!({"id":id,"connectionId":connection_id,"sourceId":source.id,"agent":source.agent,"scope":source.scope,"project":source.project,"name":entry.name,"enabled":entry.definition.enabled,"transport":entry.definition.transport(),"changed":changed,"needsUpdate":needs_update,"error":entry.error,"mode":binding.map(|b|b.mode.as_str()).unwrap_or("direct")}));
                }
            }
        }
        // This describes only explicitly added project contexts. Native policy,
        // project approvals and plugin endpoint deduplication remain client-owned.
        let candidates = detected.clone();
        for row in &mut detected {
            if row["agent"] != "claude"
                || !matches!(row["scope"].as_str(), Some("user" | "local" | "project"))
            {
                continue;
            }
            let rank = |value: &Value| match value["scope"].as_str() {
                Some("local") => 3,
                Some("project") => 2,
                Some("user") => 1,
                _ => 0,
            };
            let mut overrides:Vec<_>=candidates.iter().filter(|candidate|candidate["agent"]=="claude"&&candidate["name"]==row["name"]&&rank(candidate)>rank(row)&&!candidate["project"].is_null()&&(row["project"].is_null()||candidate["project"]==row["project"]))
                .map(|candidate|json!({"sourceId":candidate["sourceId"],"project":candidate["project"],"scope":candidate["scope"]})).collect();
            overrides.sort_by_key(|value| std::cmp::Reverse(rank(value)));
            if !overrides.is_empty() {
                row["overriddenIn"] = json!(overrides);
            }
        }
        for binding in &bindings {
            if !detected
                .iter()
                .any(|row| row["sourceId"] == binding.source_id && row["name"] == binding.name)
            {
                let source = scopes.iter().find(|s| s["id"] == binding.source_id);
                let unavailable = source.is_none_or(|s| !s["error"].is_null());
                let agent = source
                    .and_then(|s| s["agent"].as_str())
                    .unwrap_or_else(|| binding.source_id.split(':').next().unwrap_or("来源"));
                detected.push(json!({"id":format!("missing:{}:{}",binding.source_id,binding.name),"connectionId":binding.connection_id,"sourceId":binding.source_id,"agent":agent,"name":binding.name,"enabled":false,"changed":true,"missing":true,"unavailable":unavailable,"mode":binding.mode}));
            }
        }
        let history = self.store.history_page(None, 25)?;
        Ok(
            json!({"agents":agents,"connections":rows,"bindings":detected,"sources":scopes,"history":history,"gateway":{"paused":self.paused.load(Ordering::SeqCst),"sessions":self.pool.count()},"platform":std::env::consts::OS,"version":env!("CARGO_PKG_VERSION")}),
        )
    }

    fn imported_bridge(&self, definition: &Definition) -> Result<Option<Connection>> {
        let Some(command) = definition.command.as_deref() else {
            return Ok(None);
        };
        let executable = Path::new(command);
        if !matches!(
            executable.file_name().and_then(|name| name.to_str()),
            Some("mcplink" | "mcplink.exe")
        ) || !definition.args.iter().any(|arg| arg == "bridge")
        {
            return Ok(None);
        }
        let (id, data_arg) = match definition.args.as_slice() {
            [bridge, flag, id] if bridge == "bridge" && flag == "--connection" => (id, None),
            [bridge, flag, id, data_flag, data]
                if bridge == "bridge" && flag == "--connection" && data_flag == "--data-dir" =>
            {
                (id, Some(data))
            }
            [bridge, data_flag, data, flag, id] | [data_flag, data, bridge, flag, id]
                if bridge == "bridge" && flag == "--connection" && data_flag == "--data-dir" =>
            {
                (id, Some(data))
            }
            _ => bail!("请保留标准 Bridge 参数，不能将包装入口导入为上游"),
        };
        if !executable.is_absolute() {
            bail!("请使用绝对路径和标准 Bridge 参数关联已有连接");
        }
        let current = std::env::current_exe().context("无法核对 Bridge 程序")?;
        if config::canonical_path(executable)? != config::canonical_path(&current)?
            && digest(&fs::read(executable).context("无法核对 Bridge 程序")?)
                != digest(&fs::read(current).context("无法核对当前程序")?)
        {
            bail!("Bridge 程序与当前构建不同，请先更新入口程序");
        }
        if definition.env.keys().any(|key| {
            !matches!(
                key.as_str(),
                "MCPLINK_DATA_DIR" | "MCPLINK_HOME" | "MCPLINK_VAULT"
            )
        }) || ["MCPLINK_HOME", "MCPLINK_VAULT"]
            .iter()
            .any(|key| definition.env.get(*key) != std::env::var(key).ok().as_ref())
        {
            bail!("Bridge 含不同的运行环境，请保留原配置并核对来源");
        }
        let data = match data_arg.or_else(|| definition.env.get("MCPLINK_DATA_DIR")) {
            Some(path) => PathBuf::from(path),
            None => directories::ProjectDirs::from("io", "mcplink", "mcplink")
                .map(|dirs| dirs.data_local_dir().to_owned())
                .context("无法定位 Bridge 默认数据目录")?,
        };
        // Vault namespaces use the supplied path spelling, even for aliases of one directory.
        if !data.is_absolute() || data.as_os_str() != self.data.as_os_str() {
            bail!("Bridge 使用其他管理数据目录，不能关联当前连接");
        }
        Ok(Some(
            self.store
                .connection(id)
                .context("Bridge 指向的托管连接不存在")?,
        ))
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
        config::check_source_import(&source, &entry.definition.original)?;
        if let Some(connection) = self.imported_bridge(&entry.definition)? {
            self.store.bind(&Binding {
                connection_id: connection.id.clone(),
                source_id: source_id.into(),
                name: name.into(),
                mode: "shared".into(),
                fingerprint: entry.fingerprint,
                revision: connection.revision,
            })?;
            return Ok(connection.public_value());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut connection = Connection {
            members: None,
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
        let binding = Binding {
            connection_id: id,
            source_id: source_id.into(),
            name: name.into(),
            mode: "direct".into(),
            fingerprint: entry.fingerprint,
            revision: 1,
        };
        crate::lifecycle::persist_definition(
            self,
            &mut connection,
            &entry.definition,
            Some(&binding),
            false,
        )?;
        Ok(connection.public_value())
    }

    fn detected_draft(&self, p: &Value) -> Result<(Detected, String)> {
        let source = self.source(required(p, "sourceId")?)?;
        let name = required(p, "sourceName")?;
        let entry = config::scan(&source)?
            .into_iter()
            .find(|e| e.name == name)
            .context("来源中的服务已变化，请刷新")?;
        if self
            .store
            .bindings()?
            .iter()
            .any(|b| b.source_id == source.id && b.name == name)
        {
            bail!("该服务已纳入托管，请刷新后编辑");
        }
        if let Some(error) = &entry.error {
            bail!("来源配置无效：{error}");
        }
        config::check_source_import(&source, &entry.definition.original)?;
        if self.imported_bridge(&entry.definition)?.is_some() {
            bail!("这是已有托管连接的共享入口，请在服务详情点击“纳入托管”关联后编辑");
        }
        let version = digest(&serde_json::to_vec(&(
            self.draft_key,
            &source.id,
            name,
            &entry.fingerprint,
        ))?);
        Ok((entry, version))
    }

    async fn save(&self, p: &Value) -> Result<Value> {
        let name = required(p, "name")?;
        config::safe_name(name)?;
        let mut definition: Definition =
            serde_json::from_value(p.get("definition").cloned().context("缺少服务配置")?)
                .context("服务配置格式有误")?;
        definition.enabled = p
            .pointer("/definition/enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let existing = p
            .get("id")
            .and_then(Value::as_str)
            .map(|id| self.store.connection(id))
            .transpose()?;
        if existing.as_ref().is_some_and(|c| c.members.is_some()) {
            bail!("请通过统一入口管理服务选择，不能将聚合入口改为上游服务");
        }
        let draft = p
            .get("sourceId")
            .and_then(Value::as_str)
            .map(|_| self.detected_draft(p))
            .transpose()?;
        let edit_version = p.pointer("/definition/editVersion");
        if draft.is_some() && (existing.is_some() || edit_version.is_none()) {
            bail!("请重新打开来源配置以保存托管连接");
        }
        let expected_version = draft
            .as_ref()
            .map(|(_, v)| v.clone())
            .or_else(|| existing.as_ref().map(Connection::edit_version));
        if let Some(version) = edit_version
            && expected_version.as_deref() != Some(version.as_str().context("无效的编辑版本")?)
        {
            bail!("连接已更新或编辑版本不匹配，请重新打开编辑窗口");
        }
        let protected_url = definition.url.as_deref().is_some_and(|url| {
            url.contains("••••") || url.to_ascii_lowercase().contains("%e2%80%a2%e2%80%a2")
        });
        if existing.is_none()
            && draft.is_none()
            && (protected_url
                || definition.args.iter().any(|v| v.starts_with("••••••••"))
                || definition
                    .headers
                    .values()
                    .chain(definition.env.values())
                    .any(|v| v == "••••••••")
                || definition.client_secret.as_deref() == Some("••••••••"))
        {
            bail!("新连接需要填写实际配置，不能使用已存值标记");
        }
        let previous = match &draft {
            Some((entry, _)) => Some(entry.definition.clone()),
            None => existing
                .as_ref()
                .map(|old| self.definition(old))
                .transpose()?,
        };
        let mut changed_origin = false;
        if let Some(old_definition) = &previous {
            if protected_url {
                if edit_version.is_none()
                    || old_definition.public_value()["url"].as_str() != definition.url.as_deref()
                {
                    bail!("地址含无效的保留标记，请重新读取连接或填写完整地址");
                }
                definition.url.clone_from(&old_definition.url);
            }
            changed_origin = definition.changed_origin(old_definition);
            if changed_origin {
                definition.discard_inherited_auth(old_definition);
            }
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
            for value in &mut definition.args {
                if value.starts_with("••••••••") {
                    let slot = value
                        .strip_prefix("••••••••[")
                        .and_then(|v| v.strip_suffix(']'))
                        .and_then(|v| v.parse::<usize>().ok())
                        .and_then(|i| i.checked_sub(1))
                        .filter(|_| edit_version.is_some())
                        .context("启动参数引用无效，请重新读取连接")?;
                    *value = old_definition
                        .args
                        .get(slot)
                        .cloned()
                        .context("启动参数引用已失效")?;
                }
            }
            definition.original.clone_from(&old_definition.original);
            definition
                .original_agent
                .clone_from(&old_definition.original_agent);
            definition
                .origin_project
                .clone_from(&old_definition.origin_project);
            if changed_origin {
                definition.clear_native_auth();
            }
        }
        config::validate(&definition)?;
        let mut connection = existing.unwrap_or_else(|| Connection {
            members: None,
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
        let same_definition = previous.as_ref() == Some(&definition);
        if !connection.definition_ref.is_empty() && same_definition {
            crate::lifecycle::retry_cleanup(self, &connection.id).await?;
            self.store.save_connection(&connection)?;
            return Ok(connection.public_value());
        }
        connection.active &= !changed_origin;
        let binding = draft.map(|(entry, _)| Binding {
            connection_id: connection.id.clone(),
            source_id: entry.source.id,
            name: entry.name,
            mode: "direct".into(),
            fingerprint: entry.fingerprint,
            revision: u64::from(same_definition),
        });
        crate::lifecycle::replace_definition(
            self,
            &mut connection,
            &definition,
            binding.as_ref(),
            crate::lifecycle::Transition::Definition,
        )
        .await?;
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
        let binding_target = p.get("bindingTarget").filter(|v| !v.is_null());
        if let Some(target) = binding_target
            && (ids.len() != 1 || targets.len() != 1 || targets[0] != required(target, "sourceId")?)
        {
            bail!("管理绑定时请选择该条目的来源");
        }
        let bindings = self.store.bindings()?;
        let mut changes = Vec::new();
        let mut virtual_files = std::collections::HashMap::<PathBuf, String>::new();
        let mut touched = std::collections::HashSet::new();
        let mut connections = Vec::new();
        for id in ids {
            let connection = self.store.connection(&id)?;
            let definition = self.definition(&connection)?;
            if action == "upsert" && mode == "direct" && connection.members.is_some() {
                bail!("聚合入口必须通过 mcplink 接入；原生直连请选择单个上游服务");
            }
            if action == "upsert"
                && mode == "shared"
                && (!connection.active
                    || (connection.members.is_none() && connection.status != "ready"))
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
                crate::discovery::require_installed(&source.agent, &self.store)?;
                if fs::symlink_metadata(&source.path)
                    .is_ok_and(|meta| meta.file_type().is_symlink())
                {
                    bail!("目标配置为符号链接，请添加真实配置文件作为来源");
                }
                let mut text = match virtual_files.get(&source.path) {
                    Some(text) => text.clone(),
                    None => config::read(&source)?.0,
                };
                if p.get("replaceShared").and_then(Value::as_bool) == Some(true) {
                    if action != "upsert" || mode != "shared" || connection.members.is_none() {
                        bail!("仅聚合入口接入支持替换单服务共享配置");
                    }
                    for previous in bindings.iter().filter(|b| {
                        b.source_id == source.id
                            && b.mode == "shared"
                            && connection.members.as_ref().is_some_and(|members| {
                                members.iter().any(|m| m.connection_id == b.connection_id)
                            })
                    }) {
                        let document = config::parse(&source, &text)?;
                        let before = document
                            .pointer("/mcp_servers")
                            .and_then(|s| s.get(&previous.name))
                            .cloned();
                        if config::fingerprint(before.as_ref()) != previous.fingerprint
                            && before.is_some()
                        {
                            bail!("待替换的单服务入口有外部修改，请先核对配置");
                        }
                        if !touched.insert((
                            source.path.clone(),
                            source.scope.clone(),
                            source.project.clone(),
                            config::name_key(&previous.name),
                        )) {
                            bail!("替换单服务入口时存在重复目标");
                        }
                        let rendered = if before.is_some() {
                            config::render(&source, &text, &previous.name, None)?
                        } else {
                            text.clone()
                        };
                        changes.push(Change {
                            id: uuid::Uuid::new_v4().to_string(),
                            connection_id: previous.connection_id.clone(),
                            source_id: source.id.clone(),
                            name: previous.name.clone(),
                            mode: "shared".into(),
                            action: "remove".into(),
                            expected_hash: digest(text.as_bytes()),
                            revision: self.store.connection(&previous.connection_id)?.revision,
                            previous_binding: Some(previous.clone()),
                            entry_version: Some(
                                self.store.entry_version(&source.id, &previous.name)?,
                            ),
                            restore_unmanaged: false,
                            before,
                            after: None,
                        });
                        text = rendered;
                    }
                }
                let document = config::parse(&source, &text)?;
                let binding = bindings.iter().find(|b| {
                    b.connection_id == connection.id
                        && b.source_id == source.id
                        && binding_target.is_none_or(|target| target["name"] == b.name)
                });
                if binding_target.is_some() && binding.is_none() {
                    bail!("绑定已变化，请重新打开管理窗口");
                }
                let name = binding
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| connection.name.clone());
                if !touched.insert((
                    source.path.clone(),
                    source.scope.clone(),
                    source.project.clone(),
                    config::name_key(&name),
                )) {
                    bail!("批量操作中存在重复目标或接入名称冲突");
                }
                let before = document
                    .get("mcp_servers")
                    .and_then(|servers| servers.get(&name))
                    .cloned();
                if binding.is_none()
                    && bindings
                        .iter()
                        .any(|b| b.source_id == source.id && b.name == name)
                {
                    bail!("目标条目已由其他连接管理");
                }
                if action == "remove" && before.is_none() && binding.is_none() {
                    continue;
                }
                if let Some(binding) = binding {
                    if config::fingerprint(before.as_ref()) != binding.fingerprint
                        && !(before.is_none() && matches!(action, "upsert" | "remove"))
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
                        let bridge = self.bridge_config(
                            &connection.id,
                            p.get("_bridgeExecutable").and_then(Value::as_str),
                        )?;
                        config::native(&serde_json::from_value(bridge)?, &source)?
                    } else {
                        config::native(&definition, &source)?
                    };
                    if action == "disable" {
                        config::set_native_enabled(&mut value, &source, false)?;
                    }
                    if action == "enable" {
                        config::set_native_enabled(&mut value, &source, true)?;
                    }
                    if source.agent == Agent::Pi {
                        crate::pi_config::preserve_presentation(&mut value, before.as_ref())?;
                    }
                    Some(value)
                };
                let rendered = if before.is_none() && after.is_none() {
                    text.clone()
                } else {
                    config::render(&source, &text, &name, after.as_ref())?
                };
                virtual_files.insert(source.path.clone(), rendered);
                changes.push(Change {
                    id: uuid::Uuid::new_v4().to_string(),
                    connection_id: connection.id.clone(),
                    entry_version: Some(self.store.entry_version(&source.id, &name)?),
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
        let summary = plan
            .changes
            .iter()
            .map(|change| self.change_summary(change))
            .collect::<Result<Vec<_>>>()?;
        let mut display = connections[0].public_value();
        if connections.len() > 1 {
            display["title"] = json!(format!("{} 项服务", connections.len()));
        }
        Ok(
            json!({"planId":plan.id,"connection":display,"changes":summary,"expiresAt":plan.created_at+600}),
        )
    }

    fn change_summary(&self, change: &Change) -> Result<Value> {
        let source = self.source(&change.source_id)?;
        let normalize = |raw: &Value| config::normalize_source(raw, &source).ok();
        let before = change.before.as_ref().and_then(normalize);
        let after = change.after.as_ref().and_then(normalize);
        let mut fields = Vec::new();
        if let (Some(old), Some(new)) = (&before, &after) {
            let a = serde_json::to_value(old)?;
            let b = serde_json::to_value(new)?;
            for field in old.public_value().as_object().context("配置无效")?.keys() {
                if a[field] == b[field] {
                    continue;
                }
                if matches!(field.as_str(), "headers" | "env") {
                    let keys: std::collections::BTreeSet<_> =
                        [a[field].as_object(), b[field].as_object()]
                            .into_iter()
                            .flatten()
                            .flat_map(|map| map.keys())
                            .collect();
                    for key in keys {
                        let previous = a[field].get(key);
                        let next = b[field].get(key);
                        if previous != next {
                            let kind = match (previous, next) {
                                (None, _) => "add",
                                (_, None) => "remove",
                                _ => "update",
                            };
                            fields.push(json!({"field":field,"key":key,"kind":kind}));
                        }
                    }
                } else {
                    fields.push(json!({"field":field,"kind":"update"}));
                }
            }
        }
        if fields.is_empty()
            && change.before.is_some()
            && change.after.is_some()
            && change.before != change.after
        {
            fields.push(json!({"field":"native","kind":"update"}));
        }
        Ok(
            json!({"id":change.id,"sourceId":source.id,"agent":source.agent,"sourceLabel":source.label,"path":source.path,"name":change.name,
            "mode":change.mode,"action":change.action,"kind":change.kind(),"fields":fields,
            "before":before.map(|d|d.public_value()),"after":after.map(|d|d.public_value()),
            "reload":"配置写入后，请在对应 Agent 重新加载 MCP 或重新开启会话"}),
        )
    }

    fn export_config(&self, p: &Value) -> Result<Value> {
        let agent: Agent =
            serde_json::from_value(p["targetAgent"].clone()).context("请选择导入客户端")?;
        if !agent.supports_import() {
            bail!("此客户端使用原生配置预览，请选择千问办公或豆包工作导入")
        }
        let mode = required(p, "mode")?;
        if !matches!(mode, "direct" | "shared") {
            bail!("接入方式无效")
        }
        let ids = p["connectionIds"]
            .as_array()
            .filter(|v| !v.is_empty())
            .context("请选择服务")?;
        let mut names = std::collections::HashSet::new();
        let mut servers = serde_json::Map::new();
        let mut entries = Vec::new();
        for id in ids {
            let connection = self
                .store
                .connection(id.as_str().context("服务标识无效")?)?;
            if !connection.active {
                bail!("连接已断开，请先重新连接")
            }
            if mode == "direct" && connection.members.is_some() {
                bail!("聚合入口必须通过 mcplink 接入，不能导出为上游直连配置");
            }
            if !names.insert(config::name_key(&connection.name)) {
                bail!("导入服务名称重复，请分开导出或先修改接入名称")
            }
            let def = if mode == "shared" {
                if connection.members.is_none() && connection.status != "ready" {
                    bail!("请先检查共享连接并完成账号认证")
                }
                serde_json::from_value(
                    self.bridge_config(&connection.id, p["_bridgeExecutable"].as_str())?,
                )?
            } else {
                self.definition(&connection)?
            };
            let entry = crate::client_import::entry(&def, &agent)?;
            entries.push(json!({"name":connection.name,"definition":entry}));
            servers.insert(connection.name, entry);
        }
        Ok(json!({"agent":agent,"mode":mode,"entries":entries,
            "config":if agent == Agent::Qwenwork { json!({"mcpServers":servers}) } else { Value::Null },
            "gatewayPaused":self.paused.load(Ordering::SeqCst),"requiresImport":true}))
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
        if self
            .store
            .pending_operations()?
            .iter()
            .any(|op| op["sourceId"] == change.source_id && op["name"] == change.name)
        {
            bail!("此配置已有写入待核对，请检查历史并重试");
        }
        if change.action == "restore" && change.after.is_some() && !change.restore_unmanaged {
            self.store
                .connection(&change.connection_id)
                .context("原连接已删除，无法恢复绑定")?;
        }
        if change.action != "restore"
            && change.revision != 0
            && self.store.connection(&change.connection_id)?.revision != change.revision
        {
            bail!("连接已更新，请重新预览");
        }
        let current_binding = self
            .store
            .bindings()?
            .into_iter()
            .find(|b| b.source_id == change.source_id && b.name == change.name);
        let version = self.store.entry_version(&change.source_id, &change.name)?;
        if current_binding != change.previous_binding
            || change.entry_version.is_some_and(|v| v != version)
        {
            bail!("目标绑定或变更记录已更新，请重新预览");
        }
        let source = self.source(&change.source_id)?;
        if source.read_only {
            bail!("该来源为只读，请核对配置位置");
        }
        crate::discovery::require_installed(&source.agent, &self.store)?;
        let (text, _) = config::read(&source)?;
        if digest(text.as_bytes()) != change.expected_hash {
            bail!("目标文件已变化，请重新预览");
        }
        if change.before.is_none() && change.after.is_none() {
            let reference = format!("change/{}", change.id);
            self.vault
                .set(&reference, &serde_json::to_string(change)?)?;
            self.store
                .record_operation(change, plan_id, &reference, "")?;
            return self.finish_change(change, plan_id);
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
            crate::vault::write_config(&source.path, rendered.as_bytes())?;
            Ok(())
        })();
        if let Err(error) = result {
            self.store
                .finish_operation(&change.id, "failed", Some(&error.to_string()))?;
            return Err(error);
        }
        // From this point the file is committed. Keep its intent pending until
        // binding, history and (for a restore) the original operation commit together.
        if std::env::var("MCPLINK_FAILPOINT").ok().as_deref() == Some("after-write")
            && std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture")
        {
            std::process::exit(93);
        }
        self.finish_change(change, plan_id)
            .context("配置已写入，绑定待核对；可重试计划或重启后台")
    }

    fn finish_change(&self, change: &Change, plan: &str) -> Result<()> {
        let binding = if change.after.is_some() && !change.restore_unmanaged {
            Some(Binding {
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
            })
        } else {
            None
        };
        self.store.commit_change(change, binding.as_ref(), plan)
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
        self.reconcile()?;
        let mut results = Vec::new();
        for change in &plan.changes {
            let (status, message) = if let Some(done) = self.store.find_operation(&change.id)? {
                (
                    done["status"].clone(),
                    "该目标已有执行记录，请核对历史结果".into(),
                )
            } else {
                match self.apply_change(change, id) {
                    Ok(()) if change.kind() == "cleanup" => {
                        (json!("applied"), "已清理失效绑定，原配置文件保留".into())
                    }
                    Ok(()) => (json!("applied"), "配置已写入，等待 Agent 重新加载".into()),
                    Err(error) => {
                        let status = self
                            .store
                            .find_operation(&change.id)?
                            .map(|op| op["status"].clone())
                            .unwrap_or(json!("failed"));
                        (status, error.to_string())
                    }
                }
            };
            results.push(json!({"sourceId":change.source_id,"name":change.name,"status":status,"operationId":change.id,"message":message}));
        }
        Ok(json!({"results":results}))
    }

    fn restore(&self, id: &str, preview: bool) -> Result<Value> {
        if !preview {
            self.reconcile()?;
        }
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
        if operation["status"] == "pending" {
            bail!("此操作的绑定仍待核对");
        }
        let (text, document) = config::read(&source)?;
        let current = document["mcp_servers"].get(&original.name).cloned();
        let binding = self
            .store
            .bindings()?
            .into_iter()
            .find(|b| b.source_id == source.id && b.name == original.name);
        let owner = original
            .after
            .as_ref()
            .filter(|_| !original.restore_unmanaged)
            .map(|_| original.connection_id.as_str());
        if binding.as_ref().map(|b| b.connection_id.as_str()) != owner
            || !self.store.can_restore(id)?
        {
            let owner = binding
                .as_ref()
                .and_then(|b| self.store.connection(&b.connection_id).ok())
                .map(|c| c.title)
                .unwrap_or_else(|| "当前未绑定".into());
            bail!("该条目的绑定或操作有后续修改（{owner}），请从最新记录核对");
        }
        if current != original.after {
            bail!("该条目有后续修改，请核对差异后处理，其他配置保持当前版本");
        }
        let effect = original.restore_effect();
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
            previous_binding: binding,
            entry_version: None,
            restore_unmanaged: original.previous_binding.is_none(),
            action: "restore".into(),
            expected_hash: digest(text.as_bytes()),
            before: current,
            after: original.before,
        };
        if preview {
            return Ok(json!({"change":self.change_summary(&reversed)?,"effect":effect}));
        }
        self.apply_change(&reversed, &format!("restore/{id}"))?;
        Ok(json!({"status":"restored","operationId":reversed.id}))
    }

    fn reconcile(&self) -> Result<()> {
        for operation in self.store.pending_operations()? {
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
                    if let Err(error) = self.finish_change(
                        &change,
                        operation["planId"].as_str().context("缺少计划标识")?,
                    ) {
                        self.store.finish_operation(
                            id,
                            "pending",
                            Some(&format!("绑定待核对：{error}")),
                        )?;
                    }
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

    fn begin_connection_deletion(&self, id: &str, keep_native: bool) -> Result<ConnectionDeletion> {
        if self.store.connections()?.iter().any(|c| {
            c.members
                .as_ref()
                .is_some_and(|members| members.iter().any(|m| m.connection_id == id))
        }) {
            bail!("此服务仍被统一入口使用，请先从入口的服务选择中移除");
        }
        if self
            .store
            .pending_operations()?
            .iter()
            .any(|op| op["connectionId"] == id)
        {
            bail!("此连接有待核对的配置写入，请先处理变更记录");
        }
        let bindings: Vec<_> = self
            .store
            .bindings()?
            .into_iter()
            .filter(|b| b.connection_id == id)
            .collect();
        if !bindings.is_empty() && !keep_native {
            bail!("请先移除各 Agent 的配置绑定，再删除连接");
        }
        if bindings.iter().any(|b| b.mode != "direct") {
            bail!("共享入口仍需托管连接，不能保留原生配置退出托管");
        }
        for binding in &bindings {
            let source = self.source(&binding.source_id)?;
            let entry = config::scan(&source)?
                .into_iter()
                .find(|e| e.name == binding.name)
                .context("原生直连条目已不存在，请先核对来源或解除绑定")?;
            if entry.fingerprint != binding.fingerprint {
                bail!("原生直连条目已变化，请先核对来源再退出托管");
            }
        }
        self.store.delete_connection(id)?;
        self.store
            .connection_deletion(id)?
            .context("服务连接不存在，请刷新清单")
    }

    fn finish_credential_deletion(&self, id: &str, definition_ref: &str) -> Result<()> {
        crate::lifecycle::cleanup_credentials(self, id)?;
        self.vault.delete(&format!("oauth/{id}"))?;
        self.vault.delete(definition_ref)?;
        self.store.finish_credential_deletion(id)
    }

    async fn delete_connection(&self, id: &str, keep_native: bool) -> Result<Value> {
        let deletion = match self.store.connection_deletion(id)? {
            Some(deletion) => deletion,
            None => self.begin_connection_deletion(id, keep_native)?,
        };
        if let Some(definition_ref) = &deletion.definition_ref {
            let edit = crate::lifecycle::CredentialEdit::begin(self, id).await;
            let result = self
                .finish_credential_deletion(id, definition_ref)
                .context("托管记录已删除，凭据清理待完成。恢复凭据库访问后可用原命令重试");
            edit.finish(result).await?;
        }
        Ok(if keep_native {
            json!({"deleted":true,"nativePreserved":true,"releasedBindings":deletion.released_bindings})
        } else {
            json!({"deleted":true})
        })
    }

    pub async fn rpc(self: &Arc<Self>, method: &str, p: Value) -> Result<Value> {
        if *self.shutdown.borrow() && !matches!(method, "gateway.status" | "gateway.stop") {
            bail!("共享后台正在退出，请在退出完成后重新连接");
        }
        if (method.starts_with("credentials.") || method.starts_with("oauth."))
            && p.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| self.store.connection(id).is_ok_and(|c| c.members.is_some()))
        {
            bail!("统一入口不保存账号授权，请在上游服务的账号中维护认证");
        }
        match method {
            "gateway.list" => Ok(json!(
                self.store
                    .connections()?
                    .iter()
                    .filter(|c| c.members.is_some())
                    .map(Connection::public_value)
                    .collect::<Vec<_>>()
            )),
            "gateway.save" => {
                let _guard = self.mutation.lock().await;
                crate::gateway::save(self, &p)
            }
            "mcp.catalog" => crate::gateway::catalog_version(self, required(&p, "id")?),
            "cli.generate" => {
                let names: Vec<String> =
                    serde_json::from_value(p["tools"].clone()).context("请选择工具名称数组")?;
                crate::generated::generate_managed(
                    self.clone(),
                    required(&p, "id")?,
                    required(&p, "name")?.to_owned(),
                    Path::new(required(&p, "output")?),
                    &names,
                )
                .await
            }
            "cli.inspect" => Ok(serde_json::to_value(crate::generated::Manifest::read(
                Path::new(required(&p, "manifest")?),
            )?)?),
            "cli.check" => {
                crate::generated::check_managed(self.clone(), Path::new(required(&p, "manifest")?))
                    .await
            }
            "gateway.refresh" => {
                let id = required(&p, "id")?;
                self.store.connection(id)?;
                self.pool.refresh_catalog(id);
                Ok(json!({"refreshed":true,"connectionId":id}))
            }
            "snapshot" => self.snapshot(),
            "changes.reconcile" => {
                let _guard = self.mutation.lock().await;
                self.reconcile()?;
                Ok(json!({"remaining":self.store.pending_operations()?.len()}))
            }
            "changes.history" => {
                let before = p
                    .get("before")
                    .filter(|v| !v.is_null())
                    .map(|v| v.as_i64().filter(|v| *v > 0).context("历史游标无效"))
                    .transpose()?;
                let mut page = self.store.history_page(before, 25)?;
                for row in &mut page {
                    if row["kind"].is_null() {
                        let op = self.store.operation(required(row, "id")?)?;
                        if let Some(raw) = self.vault.get(required(&op, "credentialRef")?)?
                            && let Ok(change) = serde_json::from_str::<Change>(&raw)
                        {
                            row["kind"] = json!(change.kind());
                            row["restoreEffect"] = json!(change.restore_effect());
                            self.store.cache_operation_details(&change)?;
                        }
                    }
                }
                Ok(json!(page))
            }
            "workspace.identity" => Ok(json!({"id":self.dataset_id().await?})),
            "agents.launcher" => {
                let agent: Agent =
                    serde_json::from_value(p["agent"].clone()).context("无效客户端")?;
                let path = required(&p, "path")?;
                if !crate::discovery::valid_launcher(&agent, std::path::Path::new(path)) {
                    bail!("请选择该客户端的可执行 CLI 文件");
                }
                self.store.set_launcher(&agent, path)?;
                Ok(json!({"installed":true}))
            }
            "sources.add" => {
                let _guard = self.mutation.lock().await;
                let agent: Agent =
                    serde_json::from_value(p["agent"].clone()).context("客户端类型无效")?;
                if agent != Agent::Removed && !agent.manual_import() {
                    crate::discovery::require_installed(&agent, &self.store)?;
                }
                let sources = self.sources()?;
                let source = config::source_from_params(&p, &sources)?;
                if sources.iter().any(|existing| {
                    existing.id == source.id
                        && serde_json::to_value(existing).ok() == serde_json::to_value(&source).ok()
                }) {
                    return Ok(serde_json::to_value(source)?);
                }
                self.store.save_source(&source)?;
                Ok(serde_json::to_value(source)?)
            }
            "connections.import" => {
                let _guard = self.mutation.lock().await;
                self.import(required(&p, "sourceId")?, required(&p, "name")?)
            }
            "connections.detach" => {
                let _guard = self.mutation.lock().await;
                self.store.detach(
                    required(&p, "id")?,
                    required(&p, "sourceId")?,
                    required(&p, "name")?,
                )?;
                Ok(json!({"detached":true}))
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
                let source = self.source(source_id)?;
                let mut entry = config::scan(&source)?
                    .into_iter()
                    .find(|e| e.name == name)
                    .context("来源中已移除此服务")?;
                if let Some(error) = entry.error {
                    bail!("来源配置无效：{error}");
                }
                config::check_source_import(&source, &entry.definition.original)?;
                let mut connection = self.store.connection(&binding.connection_id)?;
                let previous = self.definition(&connection)?;
                if entry.definition.changed_origin(&previous) {
                    entry.definition.discard_inherited_auth(&previous);
                    entry.definition.clear_native_auth();
                    connection.active = false;
                }
                binding.fingerprint = entry.fingerprint;
                binding.revision = connection.revision + 1;
                crate::lifecycle::replace_definition(
                    self,
                    &mut connection,
                    &entry.definition,
                    Some(&binding),
                    crate::lifecycle::Transition::Definition,
                )
                .await?;
                Ok(connection.public_value())
            }
            "connections.save" => {
                let _guard = self.mutation.lock().await;
                self.save(&p).await
            }
            "connections.source_detail" => {
                let (entry, version) = self.detected_draft(&p)?;
                let mut definition = entry.definition.public_value();
                definition["editVersion"] = json!(version);
                Ok(json!({"definition":definition}))
            }
            "connections.detail" => {
                let connection = self.store.connection(required(&p, "id")?)?;
                let definition = if connection.members.is_some() {
                    Value::Null
                } else {
                    connection.editable_definition(&self.definition(&connection)?)
                };
                Ok(json!({"connection":connection.public_value(),"definition":definition}))
            }
            "connections.delete" => {
                let _guard = self.mutation.lock().await;
                let keep_native = p
                    .get("keepNative")
                    .map(|v| v.as_bool().context("keepNative 必须是布尔值"))
                    .transpose()?
                    .unwrap_or(false);
                self.delete_connection(required(&p, "id")?, keep_native)
                    .await
            }
            "connections.export_config" => {
                let _guard = self.mutation.lock().await;
                self.export_config(&p)
            }
            "changes.preview" => {
                let _guard = self.mutation.lock().await;
                self.preview(&p)
            }
            "changes.apply" => {
                let _guard = self.mutation.lock().await;
                self.apply(required(&p, "planId")?)
            }
            "changes.restore" | "changes.preview_restore" => {
                let _guard = self.mutation.lock().await;
                self.restore(
                    required(&p, "operationId")?,
                    method == "changes.preview_restore",
                )
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
                if definition.url.is_none() {
                    bail!("本地进程使用启动参数或环境变量，请编辑服务连接");
                }
                let value = required(&p, "value")?;
                reqwest::header::HeaderName::from_bytes(header.as_bytes())
                    .context("认证请求头名称无效")?;
                reqwest::header::HeaderValue::from_str(value).context("认证请求头值无效")?;
                definition
                    .headers
                    .retain(|key, _| !key.eq_ignore_ascii_case(header));
                definition
                    .headers
                    .insert(header.to_owned(), value.to_owned());
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
                crate::lifecycle::replace_definition(
                    self,
                    &mut connection,
                    &definition,
                    None,
                    crate::lifecycle::Transition::Token,
                )
                .await?;
                Ok(connection.public_value())
            }
            "connections.reconnect" => {
                let _guard = self.mutation.lock().await;
                let mut connection = self.store.connection(required(&p, "id")?)?;
                crate::lifecycle::retry_cleanup(self, &connection.id).await?;
                if !connection.active {
                    connection.active = true;
                    connection.status = "configured".into();
                    connection.checked_at = None;
                    connection.last_error = None;
                    connection.tool_count = None;
                    connection.revision += 1;
                    self.store.save_connection(&connection)?;
                }
                Ok(connection.public_value())
            }
            "credentials.disconnect" => {
                let _guard = self.mutation.lock().await;
                let mut connection = self.store.connection(required(&p, "id")?)?;
                let mut definition = self.definition(&connection)?;
                definition.headers.clear();
                definition.clear_native_auth();
                crate::lifecycle::replace_definition(
                    self,
                    &mut connection,
                    &definition,
                    None,
                    crate::lifecycle::Transition::Disconnect,
                )
                .await?;
                Ok(connection.public_value())
            }
            "oauth.start" => crate::mcp::start_oauth(self.clone(), required(&p, "id")?).await,
            "oauth.configure" => crate::mcp::configure_oauth(self.clone(), &p).await,
            "gateway.status" => Ok(
                json!({"paused":self.paused.load(Ordering::SeqCst),"sessions":self.pool.count()}),
            ),
            "gateway.pause" => {
                let _guard = self.mutation.lock().await;
                self.store.set_gateway_paused(true)?;
                self.paused.store(true, Ordering::SeqCst);
                self.pool.clear().await;
                Ok(json!({"paused":true}))
            }
            "gateway.resume" => {
                let _guard = self.mutation.lock().await;
                self.store.set_gateway_paused(false)?;
                self.paused.store(false, Ordering::SeqCst);
                Ok(json!({"paused":false}))
            }
            "gateway.stop" => {
                self.shutdown.send_replace(true);
                let _guard = self.mutation.lock().await;
                // Stopping is temporary; preserve the user's persisted pause preference.
                self.paused.store(true, Ordering::SeqCst);
                self.pool.shutdown().await?;
                Ok(json!({"stopped":true}))
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
                let connection = self.store.connection(required(&p, "id")?)?;
                if connection.members.is_some() {
                    if !connection.active {
                        bail!("统一入口已停用");
                    }
                    return Ok(crate::gateway::description());
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
        let shared = self.store.connections()?.iter().any(|c| {
            c.members
                .as_ref()
                .is_some_and(|members| members.iter().any(|m| m.connection_id == id))
        }) || self
            .store
            .bindings()?
            .iter()
            .any(|binding| binding.connection_id == id && binding.mode == "shared");
        let pool = if shared && !self.paused.load(Ordering::SeqCst) {
            &self.pool
        } else {
            &temporary
        };
        let result = crate::invocation::discover_report(self.clone(), id, pool)
            .await
            .map(|(_, tools, errors)| json!({"tools":tools,"upstreamErrors":errors}));
        temporary.shutdown().await?;
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
                connection.last_error = value["upstreamErrors"]
                    .as_array()
                    .filter(|errors| !errors.is_empty())
                    .map(|errors| {
                        format!("{} 项上游暂不可用，请检查入口服务与账号授权", errors.len())
                    });
                self.store.save_connection(&connection)?;
                Ok(
                    json!({"connection":connection.public_value(),"tools":tools,"upstreamErrors":value["upstreamErrors"],"schemaHash":digest(&serde_json::to_vec(&value)?) }),
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
