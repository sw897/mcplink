use crate::{
    model::{Binding, Connection, Source, now},
    vault::private_dir,
};
use anyhow::{Context, Result};
use rusqlite::{Connection as Sqlite, OptionalExtension, params};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};

pub struct Store {
    db: Mutex<Sqlite>,
}

pub(crate) struct ConnectionDeletion {
    pub definition_ref: Option<String>,
    pub released_bindings: i64,
}

impl Store {
    pub fn open(data: &Path) -> Result<Self> {
        private_dir(data)?;
        let db = Sqlite::open(data.join("mcplink.sqlite3")).context("无法打开本地数据库")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        let schema: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        anyhow::ensure!(schema <= 5, "数据库来自较新版本，请升级 mcplink");
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS connections(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS bindings(connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE, source_id TEXT NOT NULL, name TEXT NOT NULL, document TEXT NOT NULL, PRIMARY KEY(source_id,name));
            CREATE TABLE IF NOT EXISTS sources(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS settings(name TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY, plan_id TEXT NOT NULL, source_id TEXT NOT NULL, connection_id TEXT NOT NULL, name TEXT NOT NULL, status TEXT NOT NULL, credential_ref TEXT NOT NULL, backup TEXT, created_at INTEGER NOT NULL, error TEXT);
            CREATE INDEX IF NOT EXISTS pending_operations ON operations(status) WHERE status='pending';
            CREATE INDEX IF NOT EXISTS operations_by_entry ON operations(source_id,name);
            CREATE TABLE IF NOT EXISTS entry_ownership(source_id TEXT NOT NULL,name TEXT NOT NULL,generation INTEGER NOT NULL,history_floor INTEGER NOT NULL,PRIMARY KEY(source_id,name));
            CREATE TABLE IF NOT EXISTS operation_details(id TEXT PRIMARY KEY REFERENCES operations(id),kind TEXT NOT NULL,restore_effect TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS connection_deletions(connection_id TEXT PRIMARY KEY, definition_ref TEXT, released_bindings INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS connection_deletions_pending ON connection_deletions(connection_id) WHERE definition_ref IS NOT NULL;
            CREATE TABLE IF NOT EXISTS credential_cleanup(connection_id TEXT NOT NULL, reference TEXT PRIMARY KEY);
            PRAGMA user_version=5;")?;
        Ok(Self { db: Mutex::new(db) })
    }

    pub fn launcher(&self, agent: &crate::model::Agent) -> Result<Option<String>> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE name=?1",
                [format!("launcher.{}", agent.label())],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_launcher(&self, agent: &crate::model::Agent, path: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT OR REPLACE INTO settings(name,value) VALUES(?1,?2)",
            params![format!("launcher.{}", agent.label()), path],
        )?;
        Ok(())
    }

    pub fn gateway_paused(&self) -> Result<bool> {
        let value: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE name='gateway.paused'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| value.parse().context("无法读取共享暂停状态"))
            .unwrap_or(Ok(false))
    }

    pub fn set_gateway_paused(&self, paused: bool) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO settings(name,value) VALUES('gateway.paused',?1) ON CONFLICT(name) DO UPDATE SET value=excluded.value",
            [paused.to_string()],
        )?;
        Ok(())
    }

    pub fn connections(&self) -> Result<Vec<Connection>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT document FROM connections ORDER BY rowid")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn connection(&self, id: &str) -> Result<Connection> {
        self.connections()?
            .into_iter()
            .find(|item| item.id == id)
            .context("服务连接不存在，请刷新清单")
    }

    pub fn save_connection(&self, value: &Connection) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO connections(id,document) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document",params![value.id,serde_json::to_string(value)?])?;
        Ok(())
    }

    pub fn save_bound_connection(&self, connection: &Connection, binding: &Binding) -> Result<()> {
        self.commit_definition(connection, Some(binding), &[])
    }

    pub(crate) fn commit_definition(
        &self,
        connection: &Connection,
        binding: Option<&Binding>,
        cleanup: &[String],
    ) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO connections(id, document) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET document=excluded.document",
            params![connection.id, serde_json::to_string(connection)?],
        )?;
        if let Some(binding) = binding {
            mark_ownership(&tx, &binding.source_id, &binding.name)?;
            let updated = tx.execute(
            "INSERT INTO bindings(connection_id,source_id,name,document) VALUES(?1,?2,?3,?4)
             ON CONFLICT(source_id,name) DO UPDATE SET document=excluded.document WHERE bindings.connection_id=excluded.connection_id",
            params![
                binding.connection_id,
                binding.source_id,
                binding.name,
                serde_json::to_string(binding)?
            ],
        )?;
            anyhow::ensure!(updated == 1, "绑定已由其他连接接管");
        }
        for reference in cleanup {
            tx.execute(
                "INSERT OR IGNORE INTO credential_cleanup(connection_id, reference) VALUES(?1,?2)",
                params![connection.id, reference],
            )?;
        }
        // The new definition is now referenced. Its staging cleanup must be
        // removed atomically with this reference becoming current.
        tx.execute(
            "DELETE FROM credential_cleanup WHERE reference=?1",
            [&connection.definition_ref],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn pending_credential_cleanup(&self) -> Result<Vec<(String, String)>> {
        let db = self.db.lock().unwrap();
        let mut statement =
            db.prepare("SELECT connection_id, reference FROM credential_cleanup ORDER BY rowid")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub(crate) fn stage_definition(&self, id: &str, reference: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO credential_cleanup(connection_id, reference) VALUES(?1,?2)",
            params![id, reference],
        )?;
        Ok(())
    }

    pub(crate) fn finish_credential_cleanup(&self, reference: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "DELETE FROM credential_cleanup WHERE reference=?1",
            [reference],
        )?;
        Ok(())
    }

    pub fn delete_connection(&self, id: &str) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        {
            let mut bindings =
                tx.prepare("SELECT source_id,name FROM bindings WHERE connection_id=?1")?;
            let rows = bindings.query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (source, name) = row?;
                mark_ownership(&tx, &source, &name)?;
            }
        }
        // Keychain access is independent of SQLite. Keep the references until
        // erasure succeeds, including if the daemon exits just after commit.
        let recorded = tx.execute("INSERT INTO connection_deletions(connection_id,definition_ref,released_bindings)
            SELECT id,json_extract(document,'$.definitionRef'),(SELECT COUNT(*) FROM bindings WHERE connection_id=?1)
            FROM connections WHERE id=?1 AND json_type(document,'$.definitionRef')='text'", [id])?;
        anyhow::ensure!(recorded == 1, "服务连接不存在或凭据引用无效，请核对连接");
        tx.execute("DELETE FROM connections WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn connection_deletion(&self, id: &str) -> Result<Option<ConnectionDeletion>> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT definition_ref,released_bindings FROM connection_deletions WHERE connection_id=?1", [id],
            |r| Ok(ConnectionDeletion { definition_ref: r.get(0)?, released_bindings: r.get(1)? })
        ).optional()?)
    }

    pub(crate) fn pending_credential_deletions(&self) -> Result<Vec<(String, String)>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT connection_id,definition_ref FROM connection_deletions WHERE definition_ref IS NOT NULL")?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn finish_credential_deletion(&self, id: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE connection_deletions SET definition_ref=NULL WHERE connection_id=?1",
            [id],
        )?;
        Ok(())
    }

    pub fn bindings(&self) -> Result<Vec<Binding>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT document FROM bindings")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn bind(&self, value: &Binding) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        mark_ownership(&tx, &value.source_id, &value.name)?;
        tx.execute("INSERT INTO bindings(connection_id,source_id,name,document) VALUES(?1,?2,?3,?4) ON CONFLICT(source_id,name) DO UPDATE SET connection_id=excluded.connection_id, document=excluded.document",params![value.connection_id,value.source_id,value.name,serde_json::to_string(value)?])?;
        tx.commit()?;
        Ok(())
    }

    pub fn detach(&self, id: &str, source: &str, name: &str) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let removed = tx.execute(
            "DELETE FROM bindings WHERE source_id=?1 AND name=?2 AND connection_id=?3",
            params![source, name, id],
        )?;
        anyhow::ensure!(removed == 1, "绑定已变化，请刷新后重试");
        mark_ownership(&tx, source, name)?;
        tx.commit()?;
        Ok(())
    }

    pub fn entry_version(&self, source: &str, name: &str) -> Result<(i64, i64)> {
        Ok(self.db.lock().unwrap().query_row("SELECT COALESCE((SELECT generation FROM entry_ownership WHERE source_id=?1 AND name=?2),0), COALESCE(MAX(rowid),0) FROM operations WHERE source_id=?1 AND name=?2", params![source,name], |r| Ok((r.get(0)?,r.get(1)?)))?)
    }

    pub fn can_restore(&self, id: &str) -> Result<bool> {
        // A completed undo cancels its original operation. An undo of an older
        // operation still supersedes the requested record (including redo).
        Ok(self.db.lock().unwrap().query_row(
            "SELECT op.rowid > COALESCE(owner.history_floor,0) AND NOT EXISTS (
                SELECT 1 FROM operations later LEFT JOIN operations undone ON later.plan_id='restore/'||undone.id
                WHERE later.source_id=op.source_id AND later.name=op.name AND later.rowid>op.rowid
                AND later.status IN ('applied','pending','conflict') AND (undone.rowid IS NULL OR undone.rowid<=op.rowid))
             FROM operations op LEFT JOIN entry_ownership owner ON owner.source_id=op.source_id AND owner.name=op.name
             WHERE op.id=?1", [id], |r| r.get(0))?)
    }

    pub fn cache_operation_details(&self, change: &crate::model::Change) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO operation_details VALUES(?1,?2,?3)",
            params![change.id, change.kind(), change.restore_effect()],
        )?;
        Ok(())
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT document FROM sources ORDER BY rowid")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut sources = Vec::new();
        for row in rows {
            let source: Source = serde_json::from_str(&row?)?;
            // Retired adapters are inactive; retain their data rather than rewriting native files.
            if source.agent != crate::model::Agent::Removed {
                sources.push(source);
            }
        }
        Ok(sources)
    }

    pub fn save_source(&self, source: &Source) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT OR REPLACE INTO sources(id,document) VALUES(?1,?2)",
            params![source.id, serde_json::to_string(source)?],
        )?;
        Ok(())
    }

    pub fn operation(&self, id: &str) -> Result<Value> {
        self.find_operation(id)?.context("变更记录不存在")
    }

    pub fn find_operation(&self, id: &str) -> Result<Option<Value>> {
        let db = self.db.lock().unwrap();
        Ok(db.query_row("SELECT id,plan_id,source_id,connection_id,name,status,credential_ref FROM operations WHERE id=?1", [id], operation_row).optional()?)
    }

    pub fn record_operation(
        &self,
        change: &crate::model::Change,
        plan: &str,
        reference: &str,
        backup: &str,
    ) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO operations(id,plan_id,source_id,connection_id,name,status,credential_ref,backup,created_at) VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,?8)",params![change.id,plan,change.source_id,change.connection_id,change.name,reference,backup,now() as i64])?;
        tx.execute(
            "INSERT INTO operation_details VALUES(?1,?2,?3)",
            params![change.id, change.kind(), change.restore_effect()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish_operation(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE operations SET status=?2,error=?3 WHERE id=?1",
            params![id, status, error],
        )?;
        Ok(())
    }

    pub fn commit_change(
        &self,
        change: &crate::model::Change,
        binding: Option<&Binding>,
        plan: &str,
    ) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        if let Some(value) = binding {
            tx.execute("INSERT INTO bindings(connection_id,source_id,name,document) VALUES(?1,?2,?3,?4) ON CONFLICT(source_id,name) DO UPDATE SET connection_id=excluded.connection_id, document=excluded.document",
                params![value.connection_id,value.source_id,value.name,serde_json::to_string(value)?])?;
        } else {
            tx.execute(
                "DELETE FROM bindings WHERE source_id=?1 AND name=?2",
                params![change.source_id, change.name],
            )?;
        }
        tx.execute(
            "UPDATE operations SET status='applied',error=NULL WHERE id=?1",
            [&change.id],
        )?;
        if let Some(original) = plan.strip_prefix("restore/") {
            tx.execute(
                "UPDATE operations SET status='restored',error=NULL WHERE id=?1",
                [original],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn history_page(&self, before: Option<i64>, limit: u32) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT op.rowid,op.id,plan_id,source_id,connection_id,name,status,created_at,error,d.kind,d.restore_effect
            FROM operations op LEFT JOIN operation_details d ON d.id=op.id
            WHERE op.rowid < COALESCE(?1,9223372036854775807) ORDER BY op.rowid DESC LIMIT ?2")?;
        let rows = statement.query_map(params![before, limit], |r| Ok(json!({
            "cursor":r.get::<_,i64>(0)?, "id":r.get::<_,String>(1)?, "planId":r.get::<_,String>(2)?,
            "sourceId":r.get::<_,String>(3)?, "connectionId":r.get::<_,String>(4)?, "name":r.get::<_,String>(5)?,
            "status":r.get::<_,String>(6)?, "createdAt":r.get::<_,i64>(7)?, "error":r.get::<_,Option<String>>(8)?, "kind":r.get::<_,Option<String>>(9)?, "restoreEffect":r.get::<_,Option<String>>(10)?
        })))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending_operations(&self) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let mut statement=db.prepare("SELECT id,plan_id,source_id,connection_id,name,status,credential_ref FROM operations WHERE status='pending' ORDER BY rowid")?;
        Ok(statement
            .query_map([], operation_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
}

fn mark_ownership(tx: &rusqlite::Transaction<'_>, source: &str, name: &str) -> Result<()> {
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE source_id=?1 AND name=?2 AND status='pending')", params![source,name], |r| r.get(0))?;
    anyhow::ensure!(!pending, "此条目有待核对的配置写入");
    tx.execute("INSERT INTO entry_ownership(source_id,name,generation,history_floor)
        SELECT ?1,?2,1,COALESCE(MAX(rowid),0) FROM operations WHERE source_id=?1 AND name=?2
        ON CONFLICT(source_id,name) DO UPDATE SET generation=generation+1,history_floor=excluded.history_floor",
        params![source,name])?;
    Ok(())
}

fn operation_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>(0)?, "planId":r.get::<_,String>(1)?, "sourceId":r.get::<_,String>(2)?,
        "connectionId":r.get::<_,String>(3)?, "name":r.get::<_,String>(4)?, "status":r.get::<_,String>(5)?,
        "credentialRef":r.get::<_,String>(6)?}),
    )
}
