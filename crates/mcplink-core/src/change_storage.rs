//! Retention of config preview credentials, transaction snapshots, and file backups.
use crate::{
    Engine,
    model::{ChangePlan, now},
    vault::{canonical_uuid, private_dir, protect_private_file},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(crate) const PREVIEW_SECONDS: u64 = 600;
pub(crate) const MAX_PREVIEWS: usize = 32;
pub(crate) const PREVIEW_BUDGET: usize = 16 * 1024 * 1024;
pub(crate) const MAX_CHANGES: usize = 128;
pub(crate) const MAX_PLAN_SIZE: usize = 4 * 1024 * 1024;
pub(crate) const BACKUP_SECONDS: i64 = 30 * 24 * 3600;

impl Engine {
    pub(crate) fn save_preview(&self, plan: &ChangePlan) -> Result<()> {
        self.prune_previews()?;
        let value = serde_json::to_string(plan)?;
        ensure!(
            value.len() <= MAX_PLAN_SIZE,
            "配置预览超过 4 MiB，请分批预览"
        );
        self.store
            .stage_preview(&plan.id, plan.created_at + PREVIEW_SECONDS, value.len())?;
        self.vault.set(&format!("plan/{}", plan.id), &value)?;
        self.store.publish_preview(&plan.id)
    }

    pub(crate) fn load_preview(&self, id: &str) -> Result<Option<ChangePlan>> {
        let preview = self.store.preview(id)?;
        if let Some(preview) = preview {
            if !preview.ready || preview.expires_at <= now() {
                return Ok(None);
            }
        } else if self
            .store
            .storage_migration_deadline()?
            .is_none_or(|deadline| deadline <= now())
        {
            return Ok(None);
        }
        let Some(raw) = self.vault.get(&format!("plan/{id}"))? else {
            return Ok(None);
        };
        ensure!(raw.len() <= MAX_PLAN_SIZE, "配置预览过大，请分批重新预览");
        let plan: ChangePlan = serde_json::from_str(&raw).context("配置预览无效，请重新预览")?;
        ensure!(
            plan.id == id
                && plan.changes.len() <= MAX_CHANGES
                && plan.changes.iter().all(|change| canonical_uuid(&change.id)),
            "配置预览记录无效，请重新预览"
        );
        let time = now();
        Ok(
            (plan.created_at <= time && time < plan.created_at.saturating_add(PREVIEW_SECONDS))
                .then_some(plan),
        )
    }

    pub(crate) fn preview_result(&self, id: &str, include_legacy: bool) -> Result<Option<Value>> {
        let saved = self.store.preview(id)?.and_then(|entry| entry.result);
        if let Some(saved) = saved {
            let mut result: Value =
                serde_json::from_str(&saved).context("无法读取此计划的执行结果")?;
            for row in result["results"]
                .as_array_mut()
                .context("此计划的执行结果无效")?
            {
                if let Some(id) = row["operationId"].as_str()
                    && let Some(op) = self.store.find_operation(id)?
                    && op["status"] != row["status"]
                {
                    row["status"] = op["status"].clone();
                    row["message"] = json!("变更状态已更新，详情见操作历史");
                }
            }
            return Ok(Some(result));
        }
        if include_legacy {
            let operations = self.store.plan_operations(id)?;
            if !operations.is_empty() {
                let rows: Vec<_> = operations.iter().map(|op| json!({"sourceId":op["sourceId"],"name":op["name"],
                    "status":op["status"],"operationId":op["id"],"message":"该目标已有执行记录，请核对历史结果"})).collect();
                return Ok(Some(json!({"results":rows})));
            }
        }
        Ok(None)
    }

    pub(crate) fn prune_previews(&self) -> Result<()> {
        for id in self.store.obsolete_previews()? {
            if canonical_uuid(&id) && self.vault.delete(&format!("plan/{id}")).is_ok() {
                self.store.finish_preview_cleanup(&id)?;
            }
        }
        Ok(())
    }

    pub(crate) fn config_backup(&self, id: &str) -> PathBuf {
        self.data.join("backups").join(format!("{id}.config"))
    }

    pub(crate) fn clean_failed_changes(&self) -> Result<()> {
        for (id, backup) in self.store.obsolete_changes()? {
            if !canonical_uuid(&id) {
                continue;
            }
            if self.vault.delete(&format!("change/{id}")).is_ok()
                && (!backup || delete_backup(&self.config_backup(&id)).is_ok())
            {
                self.store.finish_change_cleanup(&id)?;
            }
        }
        Ok(())
    }

    fn prune_backups(&self) -> Result<()> {
        let rows = self.store.backups()?;
        if rows.is_empty() {
            return Ok(());
        }
        let directory = self.data.join("backups");
        if directory.exists() {
            private_dir(&directory)?;
        }
        for (id, backup, expired) in rows {
            if !canonical_uuid(&id) || Path::new(&backup) != self.config_backup(&id) {
                continue;
            }
            let path = self.config_backup(&id);
            if expired {
                if delete_backup(&path).is_ok() {
                    self.store.finish_backup_cleanup(&id)?;
                }
            } else if let Err(error) = protect_private_file(&path)
                && error
                    .downcast_ref::<io::Error>()
                    .is_none_or(|cause| cause.kind() != io::ErrorKind::NotFound)
            {
                eprintln!("mcplink: 整文件备份 {id} 的权限待核对");
            }
        }
        Ok(())
    }

    fn migrate_old_storage(&self) -> Result<()> {
        if self
            .store
            .storage_migration_deadline()?
            .is_none_or(|deadline| deadline > now())
        {
            return Ok(());
        }
        let mut remaining = false;
        for key in self.vault.old_change_keys()? {
            let Some((kind, id)) = key.split_once('/') else {
                continue;
            };
            let live = if kind == "plan" {
                self.store.preview(id)?.is_some_and(|item| {
                    item.ready && item.expires_at > now() && item.result.is_none()
                })
            } else {
                self.store
                    .find_operation(id)?
                    .is_some_and(|op| op["status"] != "failed")
            };
            if !live && self.vault.delete(&key).is_err() {
                remaining = true;
            }
        }
        if !remaining {
            self.store.finish_storage_migration()?;
        }
        Ok(())
    }

    pub(crate) fn maintain_change_storage(&self) {
        for result in [
            self.prune_previews(),
            self.clean_failed_changes(),
            self.prune_backups(),
            self.migrate_old_storage(),
        ] {
            if result.is_err() {
                eprintln!("mcplink: 配置历史的本地清理待重试");
            }
        }
    }
}

fn delete_backup(path: &Path) -> Result<()> {
    // Only remove our regular, known backup in the private storage directory.
    let parent = path.parent().context("备份位置无效")?;
    match fs::symlink_metadata(parent) {
        Ok(_) => private_dir(parent)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(crate::config_file::regular(&meta), "备份位置不是普通文件"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    fs::remove_file(path)?;
    Ok(())
}
