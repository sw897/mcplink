use crate::update_signature;
use reqwest::{Client, Response, redirect};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(debug_assertions)]
use std::path::Path;
use std::time::Duration;
use tokio::sync::watch;
use url::Url;

pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
pub const RELEASES_URL: &str = "https://github.com/sw897/mcplink/releases";

#[derive(Clone)]
pub struct Catalog {
    pub current: Version,
    pub target: String,
    pub public_key: String,
    api: Url,
    downloads: Url,
    fixture: bool,
}

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: Url,
    size: u64,
    state: String,
    digest: Option<String>,
}

#[derive(Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub name: String,
    pub notes: String,
    pub published_at: String,
    pub prerelease: bool,
    pub url: String,
    #[serde(skip)]
    assets: Vec<Asset>,
}

#[derive(Clone)]
pub struct Package {
    pub manifest: Value,
    pub version: Version,
    pub url: Url,
    pub signature: String,
    pub sha256: String,
    pub bytes: u64,
}

impl Catalog {
    pub fn github(current: Version, target: String, public_key: String) -> Self {
        Self {
            current,
            target,
            public_key,
            api: Url::parse("https://api.github.com/").unwrap(),
            downloads: Url::parse("https://github.com/sw897/mcplink/releases/download/").unwrap(),
            fixture: false,
        }
    }

    /// Test-only transport; never compiled into release applications.
    #[cfg(debug_assertions)]
    pub fn fixture(
        current: Version,
        target: String,
        public_key: String,
        home: &Path,
        api: Url,
        downloads: Url,
    ) -> Result<Self, String> {
        if !home.join(".mcplink-test-fixture").is_file()
            || ![&api, &downloads].iter().all(|url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            || api.origin() != downloads.origin()
        {
            return Err("更新测试环境必须使用独立 fixture 和同一回环地址".into());
        }
        Ok(Self {
            current,
            target,
            public_key,
            api,
            downloads,
            fixture: true,
        })
    }

    pub fn client(&self) -> Result<Client, String> {
        let fixture_origin = self.fixture.then(|| self.api.origin());
        Client::builder()
            .user_agent(format!("mcplink/{}", self.current))
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .redirect(redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("too many updater redirects");
                }
                let url = attempt.url();
                let allowed = url.username().is_empty()
                    && url.password().is_none()
                    && (match fixture_origin.as_ref() {
                        Some(origin) => &url.origin() == origin,
                        None => {
                            url.scheme() == "https"
                                && url.port_or_known_default() == Some(443)
                                && matches!(
                                    url.host_str(),
                                    Some(
                                        "github.com"
                                            | "api.github.com"
                                            | "release-assets.githubusercontent.com"
                                            | "objects.githubusercontent.com"
                                            | "github-releases.githubusercontent.com"
                                    )
                                )
                        }
                    });
                if allowed {
                    attempt.follow()
                } else {
                    attempt.error("untrusted updater redirect")
                }
            }))
            .build()
            .map_err(|_| "无法建立 GitHub 更新连接".into())
    }

    fn asset<'a>(&self, release: &'a Release, name: &str) -> Result<Option<&'a Asset>, String> {
        let matching: Vec<_> = release.assets.iter().filter(|a| a.name == name).collect();
        if matching.len() > 1 {
            return Err("GitHub 发布包含重复更新文件".into());
        }
        let Some(asset) = matching.first().copied() else {
            return Ok(None);
        };
        let mut expected = self.downloads.clone();
        expected
            .path_segments_mut()
            .map_err(|_| "更新下载地址无效")?
            .pop_if_empty()
            .push(&release.tag)
            .push(name);
        if asset.state != "uploaded"
            || asset.browser_download_url != expected
            || !expected.username().is_empty()
            || expected.password().is_some()
        {
            return Err("更新文件不属于所选 GitHub 发布".into());
        }
        Ok(Some(asset))
    }

    pub async fn latest(&self, preview: bool) -> Result<Option<Release>, String> {
        let client = self.client()?;
        let mut releases = Vec::new();
        for page in 1..=5 {
            let mut url = self
                .api
                .join("repos/sw897/mcplink/releases")
                .map_err(|_| "GitHub 地址无效")?;
            url.query_pairs_mut()
                .append_pair("per_page", "100")
                .append_pair("page", &page.to_string());
            let response = client
                .get(url)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|_| "无法连接 GitHub，请检查网络后重试")?;
            let bytes = bounded(response, 4 * 1024 * 1024).await?;
            let batch: Vec<GithubRelease> =
                serde_json::from_slice(&bytes).map_err(|_| "GitHub 发布列表格式无效")?;
            let finished = batch.len() < 100;
            for release in batch {
                let Ok(version) = Version::parse(release.tag_name.trim_start_matches('v')) else {
                    continue;
                };
                if release.tag_name != version.to_string()
                    && release.tag_name != format!("v{version}")
                {
                    continue;
                }
                let is_preview = release.prerelease || !version.pre.is_empty();
                let Some(date) = release.published_at else {
                    continue;
                };
                if release.draft || (!preview && is_preview) {
                    continue;
                }
                let url = format!("{RELEASES_URL}/tag/{}", release.tag_name);
                releases.push(Release {
                    version,
                    tag: release.tag_name.clone(),
                    name: release.name.unwrap_or(release.tag_name),
                    notes: release
                        .body
                        .unwrap_or_default()
                        .chars()
                        .take(32_000)
                        .collect(),
                    published_at: date,
                    prerelease: is_preview,
                    url,
                    assets: release.assets,
                });
            }
            if finished {
                return Ok(releases
                    .into_iter()
                    .max_by(|a, b| a.version.cmp_precedence(&b.version)));
            }
        }
        Err("GitHub 发布列表超过检查范围，请打开发布页面查看".into())
    }

    pub async fn package(&self, release: &Release) -> Result<Option<Package>, String> {
        let (manifest_name, package_name) = match self.target.as_str() {
            "darwin-aarch64" => (
                "macos-updater.json",
                format!("mcplink_update_{}_aarch64.app.tar.gz", release.version),
            ),
            "windows-x86_64" => (
                "windows-updater.json",
                format!("mcplink_{}_x64-setup.exe", release.version),
            ),
            _ => return Ok(None),
        };
        let Some(manifest_asset) = self.asset(release, manifest_name)? else {
            return Ok(None);
        };
        if manifest_asset.size > 256 * 1024 {
            return Err("更新清单过大".into());
        }
        let response = self
            .client()?
            .get(manifest_asset.browser_download_url.clone())
            .send()
            .await
            .map_err(|_| "无法下载更新清单，请重试")?;
        let bytes = bounded(response, 256 * 1024).await?;
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|_| "更新清单格式无效")?;
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .and_then(|v| Version::parse(v.trim_start_matches('v')).ok())
            .ok_or("更新清单缺少有效版本")?;
        if version != release.version {
            return Err("更新清单版本与 GitHub 发布不一致".into());
        }
        let Some(platform) = manifest.get("platforms").and_then(|p| p.get(&self.target)) else {
            return Ok(None);
        };
        let Some(asset) = self.asset(release, &package_name)? else {
            return Ok(None);
        };
        let url: Url = platform
            .get("url")
            .and_then(Value::as_str)
            .ok_or("更新清单缺少下载地址")?
            .parse()
            .map_err(|_| "更新包地址无效")?;
        let signature = platform
            .get("signature")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 16_384)
            .ok_or("发布尚未提供有效更新签名")?
            .to_owned();
        let sha256 = platform
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("更新清单缺少 SHA-256")?
            .to_ascii_lowercase();
        let size = platform
            .get("size")
            .and_then(Value::as_u64)
            .ok_or("更新清单缺少文件大小")?;
        if url != asset.browser_download_url
            || size != asset.size
            || size == 0
            || size > MAX_PACKAGE_BYTES
        {
            return Err("更新包地址或大小与 GitHub 发布不一致".into());
        }
        if let Some(digest) = &asset.digest
            && digest != &format!("sha256:{sha256}")
        {
            return Err("更新包 SHA-256 与 GitHub 发布不一致".into());
        }
        Ok(Some(Package {
            manifest,
            version,
            url,
            signature,
            sha256,
            bytes: size,
        }))
    }

    pub async fn download<F: FnMut(u64, u64)>(
        &self,
        package: &Package,
        mut cancel: watch::Receiver<bool>,
        mut progress: F,
    ) -> Result<Vec<u8>, String> {
        if package.bytes == 0
            || package.bytes > MAX_PACKAGE_BYTES
            || !package.url.as_str().starts_with(self.downloads.as_str())
        {
            return Err("更新包不符合本仓库下载条件".into());
        }
        if *cancel.borrow() {
            return Err("下载已取消".into());
        }
        let request = self
            .client()?
            .get(package.url.clone())
            .header("Accept", "application/octet-stream")
            .send();
        let response = tokio::select! {
            _ = cancel.changed() => return Err("下载已取消".into()),
            result = request => result.map_err(|_| "更新包下载失败，请检查网络后重试")?,
        };
        let mut response = checked(response)?;
        if response
            .content_length()
            .is_some_and(|size| size != package.bytes)
        {
            return Err("更新包下载大小不一致".into());
        }
        let mut bytes = Vec::with_capacity(package.bytes as usize);
        loop {
            let next = tokio::select! {
                _ = cancel.changed() => return Err("下载已取消".into()),
                chunk = response.chunk() => chunk.map_err(|_| "下载中断，请重新下载更新包")?,
            };
            let Some(chunk) = next else {
                break;
            };
            if bytes.len() as u64 + chunk.len() as u64 > package.bytes {
                return Err("更新包超过声明大小".into());
            }
            bytes.extend_from_slice(&chunk);
            progress(bytes.len() as u64, package.bytes);
        }
        if bytes.len() as u64 != package.bytes || update_signature::sha256(&bytes) != package.sha256
        {
            return Err("更新包 SHA-256 或大小校验失败，未安装任何文件".into());
        }
        update_signature::verify(
            &bytes,
            &package.signature,
            &self.public_key,
            &package.version,
        )?;
        Ok(bytes)
    }
}

fn checked(response: Response) -> Result<Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    match response.status().as_u16() {
        403 | 429 => Err("GitHub 暂时限制了请求，请稍后重试".into()),
        404 => Err("GitHub 发布文件暂不可用，请稍后重新检查".into()),
        code => Err(format!("GitHub 更新请求失败（HTTP {code}），请稍后重试")),
    }
}

async fn bounded(response: Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut response = checked(response)?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("GitHub 更新响应过大".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "GitHub 更新响应中断")? {
        if bytes.len() + chunk.len() > limit {
            return Err("GitHub 更新响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
