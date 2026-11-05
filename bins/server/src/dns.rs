//! Name lookups that get past ISP blocklists.
//!
//! Some networks answer the exchanges' names with a block page — Indonesia's
//! list covers Binance and Bybit — while the exchanges themselves answer fine:
//! only the lookup lies. Exchange names are therefore looked up over HTTPS at
//! Cloudflare, then Google, reached by address so no local resolver is asked.
//! Every other name goes to the system as usual.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Names under these are looked up over HTTPS.
const EXCHANGE_DOMAINS: [&str; 13] = [
    "binance.com",
    "binance.vision",
    "bybit.com",
    "okx.com",
    "bitget.com",
    "hyperliquid.xyz",
    "kraken.com",
    "elliot.ai",
    "deribit.com",
    "gateio.ws",
    "hbdm.com",
    "bitfinex.com",
    "asterdex.com",
];
/// DNS-over-HTTPS endpoints, by address: asking a resolver for their names would defeat the point.
const ENDPOINTS: [&str; 2] = ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"];

pub fn is_exchange_host(host: &str) -> bool {
    EXCHANGE_DOMAINS.iter().any(|d| host == *d || host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')))
}

pub struct Doh {
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (Vec<IpAddr>, Instant)>>,
}

impl Doh {
    pub fn new() -> anyhow::Result<Arc<Self>> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(5)).build()?;
        Ok(Arc::new(Self { http, cache: Mutex::new(HashMap::new()) }))
    }

    /// `host`'s addresses: over HTTPS for an exchange, from the system otherwise.
    pub async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        if host.parse::<IpAddr>().is_ok() || !is_exchange_host(host) {
            return system(host).await;
        }
        if let Some(ips) = self.cache.lock().ok().and_then(|c| c.get(host).filter(|(_, until)| *until > Instant::now()).map(|(ips, _)| ips.clone())) {
            return Ok(ips);
        }
        for endpoint in ENDPOINTS {
            let Ok(url) = reqwest::Url::parse_with_params(endpoint, &[("name", host), ("type", "A")]) else { continue };
            let Ok(res) = self.http.get(url).header("accept", "application/dns-json").send().await else { continue };
            let Ok(bytes) = res.bytes().await else { continue };
            let Ok(body) = serde_json::from_slice::<Value>(&bytes) else { continue };
            let answers = body["Answer"].as_array().cloned().unwrap_or_default();
            let ips: Vec<IpAddr> = answers.iter().filter(|a| a["type"] == 1).filter_map(|a| a["data"].as_str()?.parse().ok()).collect();
            if ips.is_empty() {
                continue;
            }
            let ttl = answers.iter().filter_map(|a| a["TTL"].as_u64()).min().unwrap_or(60).clamp(30, 600);
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(host.to_string(), (ips.clone(), Instant::now() + Duration::from_secs(ttl)));
            }
            return Ok(ips);
        }
        // Both endpoints unreachable: the system's answer is still better than none.
        system(host).await
    }

    /// A reqwest resolver that looks names up this way.
    pub fn resolver(self: &Arc<Self>) -> Arc<Resolver> {
        Arc::new(Resolver(self.clone()))
    }

    /// A WebSocket to `url`, its host looked up this way.
    pub async fn connect_ws(&self, url: &str) -> anyhow::Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
        let uri: Uri = url.parse()?;
        let host = uri.host().ok_or_else(|| anyhow::anyhow!("{url} has no host"))?;
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("wss") { 443 } else { 80 });
        let ips = self.lookup(host).await.map_err(|e| anyhow::anyhow!("looking up {host}: {e}"))?;
        let mut last = None;
        for ip in ips {
            match tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(SocketAddr::new(ip, port))).await {
                Ok(Ok(stream)) => {
                    let (ws, _) = tokio_tungstenite::client_async_tls_with_config(url, stream, None, None).await?;
                    return Ok(ws);
                }
                Ok(Err(e)) => last = Some(e.to_string()),
                Err(_) => last = Some(format!("connecting to {ip} timed out")),
            }
        }
        anyhow::bail!("could not reach {host}: {}", last.unwrap_or_else(|| "no addresses".into()))
    }
}

async fn system(host: &str) -> Result<Vec<IpAddr>, String> {
    tokio::net::lookup_host((host, 0)).await.map(|addrs| addrs.map(|a| a.ip()).collect()).map_err(|e| e.to_string())
}

pub struct Resolver(Arc<Doh>);

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let doh = self.0.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let ips = doh.lookup(&host).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            Ok(Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0))) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exchange_names_go_over_https() {
        assert!(is_exchange_host("fapi.binance.com") && is_exchange_host("data-api.binance.vision") && is_exchange_host("stream.bybit.com") && is_exchange_host("bybit.com"));
        assert!(!is_exchange_host("notbinance.com"), "a suffix must be a whole label");
        assert!(!is_exchange_host("github.com") && !is_exchange_host("localhost"));
    }
}
