//! Windows generic credentials have a 2560-byte payload limit. Keep large values
//! entirely in Credential Manager; publish a manifest only after every chunk is
//! saved. Existing UTF-16 password entries remain readable without migration.

use crate::model::digest;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const LIMIT: usize = 2560;
const PREFIX: &str = "mcplink:vault:chunks:v1:";
const MAX_SIZE: usize = 64 * 1024 * 1024;

pub(super) trait Store {
    fn password(&self, key: &str) -> Result<Option<String>>;
    fn secret(&self, key: &str) -> Result<Vec<u8>>;
    fn put_password(&self, key: &str, value: &str) -> Result<()>;
    fn put_secret(&self, key: &str, value: &[u8]) -> Result<()>;
    fn remove(&self, key: &str) -> Result<()>;
}

#[cfg(windows)]
impl Store for super::Vault {
    fn password(&self, key: &str) -> Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("系统凭据库不可用或访问未获允许"),
        }
    }

    fn secret(&self, key: &str) -> Result<Vec<u8>> {
        self.entry(key)?
            .get_secret()
            .map_err(|_| anyhow::anyhow!("无法读取系统凭据库中的连接分片"))
    }

    fn put_password(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|_| anyhow::anyhow!("无法保存到系统凭据库"))
    }

    fn put_secret(&self, key: &str, value: &[u8]) -> Result<()> {
        self.entry(key)?
            .set_secret(value)
            .map_err(|_| anyhow::anyhow!("无法保存到系统凭据库"))
    }

    fn remove(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("无法清理系统凭据库中的连接"),
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Bank {
    A,
    B,
}

impl Bank {
    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    fn index_key(self, key: &str) -> String {
        let bank = match self {
            Self::A => "a",
            Self::B => "b",
        };
        format!("{key}/chunks-v1/{bank}/index")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    bank: Bank,
    size: usize,
    sha256: String,
}

impl Generation {
    fn count(&self) -> usize {
        self.size.div_ceil(LIMIT).max(1)
    }

    fn key(&self, key: &str, index: usize) -> String {
        format!("{}/{index}", self.bank.index_key(key))
    }

    fn validate(&self) -> Result<()> {
        if self.size > MAX_SIZE
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("系统凭据库中的连接分片索引无效");
        }
        Ok(())
    }
}

fn decode(value: &str) -> Result<Generation> {
    let generation: Generation = serde_json::from_str(value)
        .map_err(|_| anyhow::anyhow!("系统凭据库中的连接分片索引无效"))?;
    generation.validate()?;
    Ok(generation)
}

fn manifest(value: &str) -> Result<Option<Generation>> {
    value.strip_prefix(PREFIX).map(decode).transpose()
}

// Every staged bank has a fixed, discoverable index, saved before its chunks.
// Failed rollback never loses the locations needed by the next set or delete.
fn remove_bank(
    store: &impl Store,
    key: &str,
    bank: Bank,
    fallback: Option<&Generation>,
) -> Result<()> {
    let saved = store
        .password(&bank.index_key(key))?
        .as_deref()
        .map(decode)
        .transpose()?;
    let Some(generation) = saved.as_ref().or(fallback.filter(|g| g.bank == bank)) else {
        return Ok(());
    };
    if generation.bank != bank {
        bail!("系统凭据库中的连接分片索引无效");
    }
    let mut failed = false;
    for index in 0..generation.count() {
        failed |= store.remove(&generation.key(key, index)).is_err();
    }
    if failed {
        bail!("无法清理系统凭据库中的连接分片");
    }
    store.remove(&bank.index_key(key))
}

// The daemon owns the data directory across processes. Serialize vault access
// inside it as OAuth refresh and management requests can run concurrently.
static ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn access() -> Result<std::sync::MutexGuard<'static, ()>> {
    ACCESS
        .lock()
        .map_err(|_| anyhow::anyhow!("系统凭据库访问未能完成"))
}

pub(super) fn get(store: &impl Store, key: &str) -> Result<Option<String>> {
    let _access = access()?;
    let Some(value) = store.password(key)? else {
        return Ok(None);
    };
    let Some(current) = manifest(&value)? else {
        return Ok(Some(value));
    };
    let mut bytes = Vec::with_capacity(current.size);
    for index in 0..current.count() {
        let chunk = store.secret(&current.key(key, index))?;
        let expected = (current.size - bytes.len()).min(LIMIT);
        if chunk.len() != expected {
            bail!("系统凭据库中的连接分片不完整");
        }
        bytes.extend_from_slice(&chunk);
    }
    if digest(&bytes) != current.sha256 {
        bail!("系统凭据库中的连接分片校验失败");
    }
    Ok(Some(
        String::from_utf8(bytes).context("系统凭据库中的连接编码无效")?,
    ))
}

pub(super) fn set(store: &impl Store, key: &str, value: &str) -> Result<()> {
    let _access = access()?;
    let previous = store
        .password(key)?
        .as_deref()
        .map(manifest)
        .transpose()?
        .flatten();
    if value.encode_utf16().count() <= LIMIT / 2 {
        store.put_password(key, value)?;
        // Cleanup failure is recoverable: both bank indexes have fixed names.
        for bank in [Bank::A, Bank::B] {
            let _ = remove_bank(store, key, bank, previous.as_ref());
        }
        return Ok(());
    }
    if value.len() > MAX_SIZE {
        bail!("连接定义超出系统凭据库的保存范围");
    }
    let bank = previous.as_ref().map_or(Bank::A, |g| g.bank.other());
    remove_bank(store, key, bank, None)?;
    let current = Generation {
        bank,
        size: value.len(),
        sha256: digest(value.as_bytes()),
    };
    let encoded = serde_json::to_string(&current)?;
    store.put_password(&bank.index_key(key), &encoded)?;
    let result = (|| {
        for index in 0..current.count() {
            let start = index * LIMIT;
            let end = (start + LIMIT).min(value.len());
            store.put_secret(&current.key(key, index), &value.as_bytes()[start..end])?;
        }
        store.put_password(key, &format!("{PREFIX}{encoded}"))
    })();
    if result.is_err() {
        let _ = remove_bank(store, key, bank, None);
        return result;
    }
    let _ = remove_bank(store, key, bank.other(), previous.as_ref());
    Ok(())
}

pub(super) fn delete(store: &impl Store, key: &str) -> Result<()> {
    let _access = access()?;
    let current = store
        .password(key)?
        .as_deref()
        .map(manifest)
        .transpose()?
        .flatten();
    let mut failed = false;
    for bank in [Bank::A, Bank::B] {
        failed |= remove_bank(store, key, bank, current.as_ref()).is_err();
    }
    if failed {
        bail!("无法清理系统凭据库中的连接分片");
    }
    store.remove(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};

    #[derive(Default)]
    struct Memory {
        passwords: RefCell<BTreeMap<String, String>>,
        secrets: RefCell<BTreeMap<String, Vec<u8>>>,
        fail_chunk: RefCell<Option<usize>>,
        fail_publish: RefCell<bool>,
        fail_remove: RefCell<bool>,
    }

    impl Store for Memory {
        fn password(&self, key: &str) -> Result<Option<String>> {
            Ok(self.passwords.borrow().get(key).cloned())
        }
        fn secret(&self, key: &str) -> Result<Vec<u8>> {
            self.secrets.borrow().get(key).cloned().context("missing")
        }
        fn put_password(&self, key: &str, value: &str) -> Result<()> {
            if *self.fail_publish.borrow() && key == "definition" {
                bail!("synthetic publish failure");
            }
            assert!(value.encode_utf16().count() * 2 <= LIMIT);
            self.passwords.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn put_secret(&self, key: &str, value: &[u8]) -> Result<()> {
            if self
                .fail_chunk
                .borrow()
                .is_some_and(|n| key.ends_with(&format!("/{n}")))
            {
                bail!("synthetic chunk failure");
            }
            assert!(value.len() <= LIMIT);
            self.secrets.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn remove(&self, key: &str) -> Result<()> {
            if *self.fail_remove.borrow() {
                bail!("synthetic delete failure");
            }
            self.passwords.borrow_mut().remove(key);
            self.secrets.borrow_mut().remove(key);
            Ok(())
        }
    }

    #[test]
    fn boundaries_unicode_updates_and_delete() -> Result<()> {
        let store = Memory::default();
        for value in [
            "a".repeat(1280),
            "a".repeat(1281),
            "代理🔑".repeat(2000),
            String::new(),
        ] {
            set(&store, "definition", &value)?;
            assert_eq!(get(&store, "definition")?.as_deref(), Some(value.as_str()));
            let root = store.password("definition")?.unwrap();
            if let Some(manifest) = manifest(&root)? {
                assert_eq!(store.secrets.borrow().len(), manifest.count());
            }
        }
        delete(&store, "definition")?;
        assert!(store.secrets.borrow().is_empty());
        assert_eq!(get(&store, "definition")?, None);
        delete(&store, "definition")?;
        Ok(())
    }

    #[test]
    fn failed_writes_preserve_previous_value_and_clean_new_chunks() -> Result<()> {
        let store = Memory::default();
        for initial in ["legacy".into(), "large".repeat(2000)] {
            set(&store, "definition", &initial)?;
            let before = store.secrets.borrow().clone();
            *store.fail_chunk.borrow_mut() = Some(1);
            assert!(set(&store, "definition", &"replacement".repeat(1000)).is_err());
            *store.fail_chunk.borrow_mut() = None;
            assert_eq!(get(&store, "definition")?, Some(initial.clone()));
            assert_eq!(*store.secrets.borrow(), before);
            *store.fail_publish.borrow_mut() = true;
            assert!(set(&store, "definition", &"replacement".repeat(1000)).is_err());
            *store.fail_publish.borrow_mut() = false;
            assert_eq!(get(&store, "definition")?, Some(initial));
            assert_eq!(*store.secrets.borrow(), before);
        }
        Ok(())
    }

    #[test]
    fn cleanup_failure_remains_tracked_and_delete_retries() -> Result<()> {
        let store = Memory::default();
        set(&store, "definition", &"first".repeat(1000))?;
        *store.fail_remove.borrow_mut() = true;
        let replacement = "second".repeat(1000);
        set(&store, "definition", &replacement)?;
        assert_eq!(get(&store, "definition")?, Some(replacement));
        let root = store.password("definition")?.unwrap();
        assert!(manifest(&root)?.is_some());
        assert!(store.password(&Bank::A.index_key("definition"))?.is_some());
        assert!(store.password(&Bank::B.index_key("definition"))?.is_some());
        assert!(delete(&store, "definition").is_err());
        assert!(store.password("definition")?.is_some());
        *store.fail_remove.borrow_mut() = false;
        delete(&store, "definition")?;
        assert!(store.secrets.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn failed_initial_save_and_failed_rollback_remain_discoverable() -> Result<()> {
        let store = Memory::default();
        *store.fail_chunk.borrow_mut() = Some(1);
        *store.fail_remove.borrow_mut() = true;
        assert!(set(&store, "definition", &"synthetic".repeat(1000)).is_err());
        assert_eq!(get(&store, "definition")?, None);
        assert!(store.password(&Bank::A.index_key("definition"))?.is_some());
        assert!(!store.secrets.borrow().is_empty());
        *store.fail_remove.borrow_mut() = false;
        delete(&store, "definition")?;
        assert!(store.secrets.borrow().is_empty());
        assert!(store.passwords.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn corrupt_or_missing_chunks_are_not_returned() -> Result<()> {
        let store = Memory::default();
        set(&store, "definition", &"synthetic".repeat(1000))?;
        let chunk = store.secrets.borrow().keys().next().unwrap().clone();
        store.secrets.borrow_mut().get_mut(&chunk).unwrap()[0] ^= 1;
        assert!(get(&store, "definition").is_err());
        store.secrets.borrow_mut().remove(&chunk);
        assert!(get(&store, "definition").is_err());
        assert!(!format!("{:?}", get(&store, "definition")).contains("synthetic"));
        delete(&store, "definition")?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "writes and deletes synthetic values in the real Windows Credential Manager"]
    fn native_windows_credentials_round_trip() -> Result<()> {
        struct Cleanup(super::super::Vault);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.delete("synthetic-value");
            }
        }
        let data =
            std::env::temp_dir().join(format!("mcplink-vault-check-{}", uuid::Uuid::new_v4()));
        let cleanup = Cleanup(super::super::Vault::new(&data)?);
        let vault = &cleanup.0;
        #[cfg(feature = "test-fixtures")]
        assert!(vault.fixture.is_none());
        vault
            .entry("synthetic-value")?
            .set_password("synthetic-legacy")?;
        assert_eq!(
            vault.get("synthetic-value")?.as_deref(),
            Some("synthetic-legacy")
        );
        let large = "合成凭据🔑".repeat(4000);
        vault.set("synthetic-value", &large)?;
        assert_eq!(
            vault.get("synthetic-value")?.as_deref(),
            Some(large.as_str())
        );
        vault.set("synthetic-value", "synthetic-replacement")?;
        assert_eq!(
            vault.get("synthetic-value")?.as_deref(),
            Some("synthetic-replacement")
        );
        assert_eq!(
            vault.entry("synthetic-value")?.get_password()?,
            "synthetic-replacement"
        );
        vault.set("synthetic-value", &large)?;
        let root = vault.entry("synthetic-value")?.get_password()?;
        let current = manifest(&root)?.unwrap();
        vault.delete("synthetic-value")?;
        assert!(vault.get("synthetic-value")?.is_none());
        for index in 0..current.count() {
            assert!(matches!(
                vault
                    .entry(&current.key("synthetic-value", index))?
                    .get_secret(),
                Err(keyring::Error::NoEntry)
            ));
        }
        Ok(())
    }
}
