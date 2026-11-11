//! Authentication transitions, persistence ordering and SDK grant coordination.
use crate::{
    Engine,
    model::{Binding, Connection, Definition},
};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::transport::auth::{
    AuthError, CredentialRefreshGuard, CredentialStore, StoredCredentials,
};
use std::sync::Arc;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub(crate) enum Transition {
    Definition,
    Token,
    Disconnect,
    Authorize,
}

/// Callers hold Engine::mutation. Never wait for a session slot while holding R:
/// initialize holds the slot and may enter the SDK's refresh guard (S -> R).
/// Invalidate again after releasing R, including on failure: a peer may install
/// another slot after the first invalidation removes the old map entry.
pub(crate) struct CredentialEdit<'a> {
    engine: &'a Engine,
    id: String,
    refresh: OwnedMutexGuard<()>,
}

impl<'a> CredentialEdit<'a> {
    pub async fn begin(engine: &'a Engine, id: &str) -> Self {
        engine.pool.invalidate(id).await;
        let refresh = engine.pool.refresh_lock(id).lock_owned().await;
        Self {
            engine,
            id: id.to_owned(),
            refresh,
        }
    }

    pub async fn finish<T>(self, result: Result<T>) -> Result<T> {
        drop(self.refresh);
        self.engine.pool.invalidate(&self.id).await;
        result
    }
}

pub(crate) async fn replace_definition(
    engine: &Engine,
    connection: &mut Connection,
    definition: &Definition,
    binding: Option<&Binding>,
    transition: Transition,
) -> Result<()> {
    let edit = CredentialEdit::begin(engine, &connection.id).await;
    match transition {
        Transition::Token | Transition::Authorize => connection.active = true,
        Transition::Disconnect => connection.active = false,
        Transition::Definition => {}
    }
    if !matches!(transition, Transition::Disconnect) {
        connection.auth_kind = if definition.oauth {
            "oauth"
        } else if !definition.headers.is_empty() {
            "token"
        } else {
            "none"
        }
        .into();
    }
    connection.status = if !connection.active {
        "disconnected"
    } else if matches!(transition, Transition::Authorize) {
        "authorizing"
    } else {
        "configured"
    }
    .into();
    connection.checked_at = None;
    connection.tool_count = None;
    connection.last_error = None;
    let mut result = persist_definition(
        engine,
        connection,
        definition,
        binding,
        !matches!(transition, Transition::Definition),
    );
    if result.is_ok() && matches!(transition, Transition::Authorize) {
        connection.status = "authorizing".into();
        result = engine
            .store
            .save_connection(connection)
            .map_err(|error| error.context("OAuth 连接已保存，但授权状态保存失败；请重新登录"));
    }
    edit.finish(result).await
}

pub(crate) fn cleanup_credentials(engine: &Engine, id: &str) -> Result<()> {
    for (owner, reference) in engine.store.pending_credential_cleanup()? {
        if owner != id {
            continue;
        }
        engine.vault.delete(&reference)?;
        engine.store.finish_credential_cleanup(&reference)?;
    }
    Ok(())
}

pub(crate) async fn retry_cleanup(engine: &Engine, id: &str) -> Result<()> {
    if !engine
        .store
        .pending_credential_cleanup()?
        .iter()
        .any(|(owner, _)| owner == id)
    {
        return Ok(());
    }
    let edit = CredentialEdit::begin(engine, id).await;
    let result = cleanup_credentials(engine, id).map_err(|error| {
        error.context("先前的连接配置已保存，但旧凭据清理仍未完成；请恢复凭据库访问后重试")
    });
    edit.finish(result).await
}

pub(crate) fn complete_oauth(engine: &Engine, id: &str, revision: u64) -> Result<()> {
    let mut connection = engine.store.connection(id)?;
    anyhow::ensure!(
        connection.active && connection.revision == revision,
        "授权会话已变化"
    );
    // Completing authorization changes the observed account state. In-flight
    // probes and tool discovery from before this commit must not overwrite it.
    connection.revision += 1;
    connection.status = "configured".into();
    connection.checked_at = None;
    connection.tool_count = None;
    connection.last_error = None;
    engine.store.save_connection(&connection)
}

fn fixture_exit(point: &str) {
    if std::env::var("MCPLINK_FAILPOINT").ok().as_deref() == Some(point)
        && std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture")
        && std::env::var_os("MCPLINK_HOME").is_some_and(|home| {
            std::path::PathBuf::from(home)
                .join(".mcplink-test-fixture")
                .is_file()
        })
    {
        std::process::exit(94);
    }
}

/// Vault and SQLite are separate stores. Save the new definition and commit its
/// reference before removing old credentials. A rejected DB write leaves the old
/// definition/grant usable; cleanup failures explicitly report a committed edit.
/// Imports of brand-new IDs also use this primitive without session locks.
pub(crate) fn persist_definition(
    engine: &Engine,
    connection: &mut Connection,
    definition: &Definition,
    binding: Option<&Binding>,
    clear_grant: bool,
) -> Result<()> {
    cleanup_credentials(engine, &connection.id).map_err(|error| {
        error.context("先前的连接配置已保存，但旧凭据清理仍未完成；请恢复凭据库访问后重试")
    })?;
    let old = connection.definition_ref.clone();
    let reset_oauth = clear_grant
        || if old.is_empty() {
            false
        } else {
            let previous = engine.definition(connection)?;
            previous.url != definition.url
                || previous.http_api.is_some() != definition.http_api.is_some()
                || previous.origin_project != definition.origin_project
                || previous.oauth != definition.oauth
                || previous.client_id != definition.client_id
                || previous.client_secret != definition.client_secret
                || previous.oauth_scopes != definition.oauth_scopes
                || previous.oauth_callback_port != definition.oauth_callback_port
        };
    if reset_oauth && definition.oauth && connection.active {
        connection.status = "needs_auth".into();
    }
    let reference = format!("definition/{}", uuid::Uuid::new_v4());
    engine.store.stage_definition(&connection.id, &reference)?;
    engine
        .vault
        .set(&reference, &serde_json::to_string(definition)?)?;
    fixture_exit("definition-after-stage");
    connection.definition_ref = reference;
    connection.revision += 1;
    connection.transport = if connection.members.is_some() {
        "aggregate"
    } else {
        definition.transport()
    }
    .into();
    connection.endpoint = connection
        .members
        .as_ref()
        .map(|members| format!("{} 项托管服务", members.len()))
        .unwrap_or_else(|| definition.summary());
    let mut cleanup = Vec::new();
    if reset_oauth {
        cleanup.push(format!("oauth/{}", connection.id));
    }
    if !old.is_empty() {
        cleanup.push(old.clone());
    }
    let write = engine
        .store
        .commit_definition(connection, binding, &cleanup);
    if let Err(error) = write {
        let cleanup = cleanup_credentials(engine, &connection.id);
        connection.definition_ref = old;
        connection.revision -= 1;
        if cleanup.is_err() {
            return Err(error.context("连接配置未保存，旧认证保持不变；未提交的新凭据清理失败"));
        }
        return Err(error);
    }
    fixture_exit("definition-after-commit");
    cleanup_credentials(engine, &connection.id).map_err(|error| {
        error.context(
            "连接配置已保存，但旧凭据清理失败；恢复凭据库访问后可重试，后台重启也会继续清理",
        )
    })?;
    Ok(())
}

pub(crate) struct VaultCredentials {
    engine: Arc<Engine>,
    id: String,
    revision: u64,
    lock: Arc<AsyncMutex<()>>,
}

impl VaultCredentials {
    pub(crate) fn new(engine: Arc<Engine>, connection: &Connection) -> Self {
        let lock = engine.pool.refresh_lock(&connection.id);
        Self {
            engine,
            id: connection.id.clone(),
            revision: connection.revision,
            lock,
        }
    }

    fn check(&self) -> std::result::Result<Connection, AuthError> {
        let current = self
            .engine
            .store
            .connection(&self.id)
            .map_err(|_| AuthError::CredentialStoreError("连接不存在".into()))?;
        if !current.active || current.revision != self.revision {
            return Err(AuthError::CredentialStoreError(
                "授权会话已变化，请重新登录".into(),
            ));
        }
        Ok(current)
    }
}

#[async_trait]
impl CredentialStore for VaultCredentials {
    async fn load(&self) -> std::result::Result<Option<StoredCredentials>, AuthError> {
        if self.check()?.status == "needs_auth" {
            return Err(AuthError::CredentialStoreError("需要重新授权".into()));
        }
        self.engine
            .vault
            .get(&format!("oauth/{}", self.id))
            .map_err(|_| AuthError::CredentialStoreError("系统凭据库不可用".into()))?
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|_| AuthError::CredentialStoreError("授权数据格式有误".into()))
            })
            .transpose()
    }
    async fn save(&self, value: StoredCredentials) -> std::result::Result<(), AuthError> {
        self.check()?;
        let value = serde_json::to_string(&value)
            .map_err(|_| AuthError::CredentialStoreError("授权数据无法编码".into()))?;
        self.engine
            .vault
            .set(&format!("oauth/{}", self.id), &value)
            .map_err(|_| AuthError::CredentialStoreError("系统凭据库不可用".into()))
    }
    async fn clear(&self) -> std::result::Result<(), AuthError> {
        self.check()?;
        self.engine
            .vault
            .delete(&format!("oauth/{}", self.id))
            .map_err(|_| AuthError::CredentialStoreError("系统凭据库不可用".into()))
    }
    async fn acquire_refresh_guard(
        &self,
    ) -> std::result::Result<Option<CredentialRefreshGuard>, AuthError> {
        Ok(Some(CredentialRefreshGuard::new(
            self.lock.clone().lock_owned().await,
        )))
    }
}
