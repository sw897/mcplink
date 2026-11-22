//! Connect-time address policy for managed HTTP and OAuth traffic.
use crate::{config, model::Definition};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Client, Method,
    dns::{Name, Resolve, Resolving},
};
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientFuture, OAuthHttpRedirectPolicy, OAuthHttpRequest,
};
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use url::{Host, Origin, Url};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Public,
    Private,
    Loopback,
    Reserved,
}

fn scope(address: IpAddr) -> Scope {
    match address.to_canonical() {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            if ip.is_loopback() {
                Scope::Loopback
            } else if ip.is_private() || (a == 100 && (64..=127).contains(&b)) {
                Scope::Private
            } else if a == 0
                || a >= 224
                || ip.is_link_local()
                || ip.is_documentation()
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && matches!(b, 18 | 19))
            {
                Scope::Reserved
            } else {
                Scope::Public
            }
        }
        IpAddr::V6(ip) => {
            let [a, b, ..] = ip.segments();
            if ip.is_loopback() {
                Scope::Loopback
            } else if ip.is_unique_local() {
                Scope::Private
            } else if a & 0xe000 != 0x2000
                || matches!(a, 0x2002 | 0x3ffe)
                || (a == 0x3fff && b <= 0x0fff)
                || (a == 0x2001 && (b <= 0x01ff || b == 0x0db8))
            {
                Scope::Reserved
            } else {
                Scope::Public
            }
        }
    }
}

fn literal_scope(url: &Url) -> Option<Scope> {
    match url.host()? {
        Host::Ipv4(ip) => Some(scope(ip.into())),
        Host::Ipv6(ip) => Some(scope(ip.into())),
        Host::Domain(host) if host.trim_end_matches('.') == "localhost" => Some(Scope::Loopback),
        Host::Domain(_) => None,
    }
}

pub(crate) fn needs_explicit_selection(url: &Url) -> bool {
    matches!(literal_scope(url), Some(Scope::Private | Scope::Loopback))
        || matches!(url.host(), Some(Host::Domain(host)) if {
            let host = host.trim_end_matches('.');
            !host.contains('.') || host.ends_with(".local") || host.ends_with(".internal") || host.ends_with(".localhost")
        })
}

pub(crate) fn check_url(url: &Url) -> Result<()> {
    let address = literal_scope(url);
    ensure!(
        url.host().is_some()
            && (url.scheme() == "https"
                || (url.scheme() == "http" && address == Some(Scope::Loopback))),
        "共享 HTTP 连接需要 HTTPS，或本机 loopback 地址"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "请通过认证字段配置凭据"
    );
    ensure!(url.fragment().is_none(), "共享 HTTP 地址不能包含片段");
    ensure!(
        address != Some(Scope::Reserved),
        "共享 HTTP 连接不能访问保留或链路本地地址"
    );
    Ok(())
}

pub(crate) fn endpoint(definition: &Definition) -> Result<Url> {
    let raw = config::resolve(
        definition.url.as_deref().context("缺少 HTTP 服务地址")?,
        definition.origin_project.as_deref(),
    )?;
    let url = Url::parse(&raw).map_err(|_| anyhow::anyhow!("HTTP 服务地址无效"))?;
    check_url(&url)?;
    #[cfg(windows)]
    if url.scheme() == "http"
        && let Some(Host::Ipv6(address)) = url.host()
        && let Some(loopback) = address.to_ipv4_mapped()
        && loopback.is_loopback()
    {
        // Winsock opens IPv6 sockets in IPv6-only mode by default.
        let mut url = url;
        url.set_ip_host(loopback.into())
            .map_err(|_| anyhow::anyhow!("本机 HTTP 地址无效"))?;
        return Ok(url);
    }
    Ok(url)
}

pub(crate) fn validate_approval(definition: &Definition) -> Result<()> {
    if let Some(approved) = &definition.private_network_origin {
        let url = endpoint(definition)?;
        ensure!(
            url.scheme() == "https" && approved == &url.origin().ascii_serialization(),
            "内网确认须为当前服务的完整 HTTPS 来源（协议、主机和端口）"
        );
    }
    Ok(())
}

struct Resolver(Scope);

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allowed = self.0;
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty()
                || !addresses.iter().all(|address| {
                    let actual = scope(address.ip());
                    actual == allowed || (allowed == Scope::Private && actual == Scope::Public)
                })
            {
                return Err(std::io::Error::other("目标 DNS 地址不符合共享 HTTP 网络策略").into());
            }
            // The connector uses exactly this checked answer, including on
            // later lookups after an idle connection has closed.
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[derive(Clone)]
pub(crate) struct Outbound {
    public: Client,
    private: Client,
    loopback: Client,
    management_port: u16,
}

impl Outbound {
    pub(crate) fn new(management_port: u16) -> Result<Self> {
        let build = |scope| {
            Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(15))
                .pool_idle_timeout(Duration::from_secs(30))
                .pool_max_idle_per_host(4)
                .dns_resolver(Arc::new(Resolver(scope)))
                .build()
                .context("无法创建共享 HTTP 客户端")
        };
        Ok(Self {
            public: build(Scope::Public)?,
            private: build(Scope::Private)?,
            loopback: build(Scope::Loopback)?,
            management_port,
        })
    }

    pub(crate) fn target(&self, definition: &Definition) -> Result<(Url, Target)> {
        let url = endpoint(definition)?;
        validate_approval(definition)?;
        let target = Target {
            outbound: self.clone(),
            primary: url.origin(),
            private: definition.private_network_origin.is_some(),
        };
        target.client(&url)?;
        Ok((url, target))
    }
}

#[derive(Clone)]
pub(crate) struct Target {
    outbound: Outbound,
    primary: Origin,
    private: bool,
}

impl Target {
    pub(crate) fn client(&self, url: &Url) -> Result<&Client> {
        check_url(url)?;
        let same_origin = url.origin() == self.primary;
        match literal_scope(url) {
            Some(Scope::Loopback) => {
                ensure!(same_origin, "仅允许访问明确配置的本机服务来源");
                ensure!(
                    url.port_or_known_default() != Some(self.outbound.management_port),
                    "共享 HTTP 连接不能访问本机管理端口"
                );
                Ok(&self.outbound.loopback)
            }
            Some(Scope::Private) if !same_origin || !self.private => {
                bail!("请先明确确认该 HTTPS 内网服务来源")
            }
            _ if self.private && same_origin => Ok(&self.outbound.private),
            _ => Ok(&self.outbound.public),
        }
    }

    pub(crate) async fn oauth(
        self,
        endpoint: Url,
        cancelled: CancellationToken,
    ) -> Result<rmcp::transport::auth::AuthorizationManager, rmcp::transport::auth::AuthError> {
        rmcp::transport::auth::AuthorizationManager::new_with_oauth_http_client(
            endpoint,
            Arc::new(OAuthClient {
                target: self,
                cancelled,
            }),
        )
        .await
    }
}

struct OAuthClient {
    target: Target,
    cancelled: CancellationToken,
}

impl OAuthHttpClient for OAuthClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let limit = request
                .timeout
                .unwrap_or(Duration::from_secs(30))
                .min(Duration::from_secs(30));
            let follow = matches!(request.redirect_policy, OAuthHttpRedirectPolicy::Follow);
            let send = async {
                let mut request = reqwest::Request::try_from(request.request)?;
                let origin = request.url().origin();
                let mut redirects = 0;
                loop {
                    let client = self.target.client(request.url())?;
                    let next = (follow && matches!(*request.method(), Method::GET | Method::HEAD))
                        .then(|| request.try_clone())
                        .flatten();
                    let response = client.execute(request).await?;
                    if let Some(mut next) = next
                        && matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
                        && let Some(location) = response.headers().get(reqwest::header::LOCATION)
                    {
                        let url = response.url().join(location.to_str()?)?;
                        ensure!(
                            url.origin() == origin && redirects < 5,
                            "OAuth 元数据重定向超出当前来源或次数限制"
                        );
                        *next.url_mut() = url;
                        request = next;
                        redirects += 1;
                        continue;
                    }
                    let mut builder = axum::http::Response::builder()
                        .status(response.status())
                        .version(response.version());
                    *builder.headers_mut().context("OAuth 响应头无效")? =
                        response.headers().clone();
                    let mut response = response;
                    let mut body = Vec::new();
                    while let Some(chunk) = response.chunk().await? {
                        ensure!(
                            chunk.len() <= 1024 * 1024 - body.len(),
                            "OAuth 响应超过 1 MiB"
                        );
                        body.extend_from_slice(&chunk);
                    }
                    return Ok::<_, anyhow::Error>(builder.body(body)?);
                }
            };
            let result = tokio::select! {
                biased;
                _ = self.cancelled.cancelled() => Err(anyhow::anyhow!("OAuth 请求已取消")),
                result = tokio::time::timeout(limit, send) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("OAuth HTTP 请求超时"))),
            };
            result.map_err(anyhow::Error::into_boxed_dyn_error)
        })
    }
}
