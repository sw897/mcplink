use crate::{
    model::{Binding, Connection, Source, now},
    vault::private_dir,
};
use anyhow::{Context, Result};
use rusqlite::{Connection as Sqlite, params};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};

pub struct Store {
    db: Mutex<Sqlite>,
}

impl Store {
    pub fn open(data: &Path) -> Result<Self> {
        private_dir(data)?;
        let db = Sqlite::open(data.join("mcplink.sqlite3")).context("无法打开本地数据库")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS connections(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS bindings(connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE, source_id TEXT NOT NULL, name TEXT NOT NULL, document TEXT NOT NULL, PRIMARY KEY(source_id,name));
            CREATE TABLE IF NOT EXISTS sources(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY, plan_id TEXT NOT NULL, source_id TEXT NOT NULL, connection_id TEXT NOT NULL, name TEXT NOT NULL, status TEXT NOT NULL, credential_ref TEXT NOT NULL, backup TEXT, created_at INTEGER NOT NULL, error TEXT);
            PRAGMA user_version=1;")?;
        Ok(Self { db: Mutex::new(db) })
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

    pub fn delete_connection(&self, id: &str) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute("DELETE FROM connections WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn bindings(&self) -> Result<Vec<Binding>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT document FROM bindings")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn bind(&self, value: &Binding) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO bindings(connection_id,source_id,name,document) VALUES(?1,?2,?3,?4) ON CONFLICT(source_id,name) DO UPDATE SET connection_id=excluded.connection_id, document=excluded.document",params![value.connection_id,value.source_id,value.name,serde_json::to_string(value)?])?;
        Ok(())
    }

    pub fn unbind(&self, source: &str, name: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "DELETE FROM bindings WHERE source_id=?1 AND name=?2",
            params![source, name],
        )?;
        Ok(())
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT document FROM sources ORDER BY rowid")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn save_source(&self, source: &Source) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT OR REPLACE INTO sources(id,document) VALUES(?1,?2)",
            params![source.id, serde_json::to_string(source)?],
        )?;
        Ok(())
    }

    pub fn operation(&self, id: &str) -> Result<Value> {
        self.history()?
            .into_iter()
            .find(|value| value["id"] == id)
            .context("变更记录不存在")
    }

    pub fn record_operation(
        &self,
        change: &crate::model::Change,
        plan: &str,
        reference: &str,
        backup: &str,
    ) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO operations(id,plan_id,source_id,connection_id,name,status,credential_ref,backup,created_at) VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,?8)",params![change.id,plan,change.source_id,change.connection_id,change.name,reference,backup,now() as i64])?;
        Ok(())
    }

    pub fn finish_operation(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE operations SET status=?2,error=?3 WHERE id=?1",
            params![id, status, error],
        )?;
        Ok(())
    }

    pub fn history(&self) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let mut statement=db.prepare("SELECT id,plan_id,source_id,connection_id,name,status,credential_ref,backup,created_at,error FROM operations ORDER BY created_at DESC,rowid DESC")?;
        let rows=statement.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"planId":r.get::<_,String>(1)?,"sourceId":r.get::<_,String>(2)?,"connectionId":r.get::<_,String>(3)?,"name":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,"credentialRef":r.get::<_,String>(6)?,"backup":r.get::<_,Option<String>>(7)?,"createdAt":r.get::<_,i64>(8)?,"error":r.get::<_,Option<String>>(9)?})))?;
        rows.map(|row| Ok(row?)).collect()
    }
}
