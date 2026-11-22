//! Authenticated, instance-scoped sessions for the loopback management channel.
use anyhow::{Context, Result, anyhow, ensure};
use aws_lc_rs::{aead, constant_time::verify_slices_are_equal, hmac, rand};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{
    sync::Mutex,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use uuid::Uuid;

// The existing fixed Keychain helper already accepts protected definition keys.
// Keep the protocol root independent of the legacy Bearer credential.
pub(crate) const VAULT_KEY: &str = "definition/runtime-auth-v1";
pub(crate) const VERSION: u64 = 1;
pub(crate) const HEADER: &str = "x-mcplink-auth";
pub(crate) const SESSION_LIFETIME: Duration = Duration::from_secs(15 * 60);
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024 + 64 * 1024;
pub(crate) const MAX_REPLY_BYTES: usize = 64 * 1024 * 1024;
const MAX_MESSAGES: u64 = 1 << 20;

fn tag_context(key: &hmac::Key, label: &[u8], fields: &[&[u8]]) -> hmac::Context {
    let mut context = hmac::Context::with_key(key);
    context.update(b"mcplink-management-api-24/auth-1\0");
    context.update(label);
    context.update(b"\0");
    for field in fields {
        context.update(field);
    }
    context
}

pub(crate) struct Root(hmac::Key);

impl Root {
    pub(crate) fn load(vault: &crate::vault::Vault, create: bool) -> Result<Self> {
        let bytes = match vault.get(VAULT_KEY)? {
            Some(value) => URL_SAFE_NO_PAD
                .decode(value)
                .context("本地通信凭据格式无效")?,
            None if create => {
                let mut bytes = vec![0; 32];
                rand::fill(&mut bytes).map_err(|_| anyhow!("无法生成本地通信凭据"))?;
                vault.set(VAULT_KEY, &URL_SAFE_NO_PAD.encode(&bytes))?;
                bytes
            }
            None => anyhow::bail!("无法取得本地通信凭据"),
        };
        ensure!(bytes.len() == 32, "本地通信凭据格式无效");
        Ok(Self(hmac::Key::new(hmac::HMAC_SHA256, &bytes)))
    }

    pub(crate) fn hello(&self, instance: Uuid, client: Uuid) -> String {
        let mac = tag_context(&self.0, b"hello", &[instance.as_bytes(), client.as_bytes()]).sign();
        format!("{client}.{}", URL_SAFE_NO_PAD.encode(mac.as_ref()))
    }

    pub(crate) fn verify_hello(&self, instance: Uuid, header: &str) -> Option<Uuid> {
        let (client, mac) = header.split_once('.')?;
        let client = Uuid::parse_str(client).ok()?;
        let mac = URL_SAFE_NO_PAD.decode(mac).ok()?;
        let expected =
            tag_context(&self.0, b"hello", &[instance.as_bytes(), client.as_bytes()]).sign();
        verify_slices_are_equal(expected.as_ref(), &mac).ok()?;
        Some(client)
    }

    pub(crate) fn server_proof(&self, instance: Uuid, client: Uuid, session: Uuid) -> String {
        let mac = tag_context(
            &self.0,
            b"server",
            &[instance.as_bytes(), client.as_bytes(), session.as_bytes()],
        )
        .sign();
        URL_SAFE_NO_PAD.encode(mac.as_ref())
    }

    pub(crate) fn verify_server(
        &self,
        instance: Uuid,
        client: Uuid,
        session: Uuid,
        proof: &str,
    ) -> Result<()> {
        let mac = URL_SAFE_NO_PAD.decode(proof).context("后台身份验证失败")?;
        let expected = tag_context(
            &self.0,
            b"server",
            &[instance.as_bytes(), client.as_bytes(), session.as_bytes()],
        )
        .sign();
        verify_slices_are_equal(expected.as_ref(), &mac).map_err(|_| anyhow!("后台身份验证失败"))
    }

    pub(crate) fn session(&self, instance: Uuid, client: Uuid, id: Uuid) -> Result<WireSession> {
        let fields = &[
            instance.as_bytes().as_slice(),
            client.as_bytes(),
            id.as_bytes(),
        ];
        let derive = |label| tag_context(&self.0, label, fields).sign();
        let aead_key = |label| {
            aead::UnboundKey::new(&aead::AES_256_GCM, derive(label).as_ref())
                .map(aead::LessSafeKey::new)
                .map_err(|_| anyhow!("无法建立本地加密会话"))
        };
        Ok(WireSession {
            id,
            hello_nonce: client,
            header: hmac::Key::new(hmac::HMAC_SHA256, derive(b"header").as_ref()),
            request: aead_key(b"request")?,
            reply: aead_key(b"reply")?,
            created: Instant::now(),
            sequence: AtomicU64::new(0),
            replay: Mutex::new(ReplayWindow::default()),
        })
    }
}

pub(crate) struct WireSession {
    pub(crate) id: Uuid,
    pub(crate) hello_nonce: Uuid,
    header: hmac::Key,
    request: aead::LessSafeKey,
    reply: aead::LessSafeKey,
    pub(crate) created: Instant,
    sequence: AtomicU64,
    replay: Mutex<ReplayWindow>,
}

impl WireSession {
    pub(crate) fn next(&self) -> Result<u64> {
        self.sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
                // Keep a session comfortably below the AES-GCM per-key message limit.
                (last < MAX_MESSAGES).then_some(last + 1)
            })
            .map(|previous| previous + 1)
            .map_err(|_| anyhow!("本地通信会话已用尽，请重新连接"))
    }

    pub(crate) fn format_header(&self, number: u64) -> String {
        let mac = hmac::sign(&self.header, &number.to_be_bytes());
        format!(
            "{}.{number}.{}",
            self.id,
            URL_SAFE_NO_PAD.encode(mac.as_ref())
        )
    }

    pub(crate) fn verify_header(&self, number: u64, mac: &str) -> bool {
        number > 0
            && number <= MAX_MESSAGES
            && URL_SAFE_NO_PAD
                .decode(mac)
                .is_ok_and(|mac| hmac::verify(&self.header, &number.to_be_bytes(), &mac).is_ok())
    }

    pub(crate) fn accept(&self, number: u64) -> bool {
        self.replay.lock().unwrap().insert(number)
    }

    pub(crate) fn seal(&self, number: u64, reply: bool, bytes: &mut Vec<u8>) -> Result<()> {
        self.key(reply)
            .seal_in_place_append_tag(nonce(number), aead::Aad::empty(), bytes)
            .map_err(|_| anyhow!("无法保护本地通信数据"))
    }

    pub(crate) fn open<'a>(
        &self,
        number: u64,
        reply: bool,
        bytes: &'a mut [u8],
    ) -> Result<&'a [u8]> {
        self.key(reply)
            .open_in_place(nonce(number), aead::Aad::empty(), bytes)
            .map(|plaintext| &*plaintext)
            .map_err(|_| anyhow!("本地通信数据验证失败"))
    }

    fn key(&self, reply: bool) -> &aead::LessSafeKey {
        if reply { &self.reply } else { &self.request }
    }
}

fn nonce(number: u64) -> aead::Nonce {
    let mut nonce = [0; aead::NONCE_LEN];
    nonce[4..].copy_from_slice(&number.to_be_bytes());
    aead::Nonce::assume_unique_for_key(nonce)
}

#[derive(Default)]
struct ReplayWindow {
    latest: u64,
    seen: u128,
}

impl ReplayWindow {
    fn insert(&mut self, number: u64) -> bool {
        if number > self.latest {
            let shift = (number - self.latest).min(u64::from(u128::BITS)) as u32;
            self.seen = self.seen.checked_shl(shift).unwrap_or(0) | 1;
            self.latest = number;
            return true;
        }
        let offset = (self.latest - number).min(u64::from(u128::BITS)) as u32;
        let Some(bit) = 1_u128.checked_shl(offset) else {
            return false;
        };
        let fresh = self.seen & bit == 0;
        self.seen |= bit;
        fresh
    }
}
