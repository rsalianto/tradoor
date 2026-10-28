//! The exchanges, for browsers that cannot reach them.
//!
//! The browser talks to the exchanges itself; this server is not on that
//! path. But on networks that block the exchanges' names — a phone on an
//! Indonesian carrier — the browser gets nowhere, while this server, looking
//! names up over HTTPS, still does. So the page falls back to these two routes:
//! the handful of REST calls charts make, and the public market streams. Both
//! are allow-listed: this is a way to the exchanges' market data, not a proxy.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ws::{Message as ClientMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::Message as UpstreamMessage;

use crate::api::{ApiError, AppState};
use crate::limits;
use crate::dns::Doh;

/// REST calls a chart makes, by host and exact path — or by a prefix ending
/// in `/`, where the market is part of the path (Kraken Futures' charts).
/// Hyperliquid's are POSTs and are not relayed.
const REST: [(&str, &[&str]); 9] = [
    // `depth`: the order book a DOM lays the stream's changes over.
    ("api.binance.com", &["/api/v3/klines", "/api/v3/ping", "/api/v3/depth"]),
    ("data-api.binance.vision", &["/api/v3/klines", "/api/v3/ping", "/api/v3/depth"]),
    ("fapi.binance.com", &["/fapi/v1/klines", "/fapi/v1/ping", "/fapi/v1/depth"]),
    ("api.bybit.com", &["/v5/market/kline", "/v5/market/time"]),
    // `instruments`: the coins in a swap contract, which its sizes count.
    ("www.okx.com", &["/api/v5/market/candles", "/api/v5/market/history-candles", "/api/v5/public/instruments"]),
    ("api.bitget.com", &["/api/v2/spot/market/candles", "/api/v2/spot/market/history-candles", "/api/v2/mix/market/candles", "/api/v2/mix/market/history-candles"]),
    ("api.kraken.com", &["/0/public/OHLC"]),
    ("futures.kraken.com", &["/api/charts/v1/trade/"]),
    ("mainnet.zklighter.elliot.ai", &["/api/v1/candles"]),
];
/// Upstream responses larger than this are refused: a kline page is a few hundred kilobytes.
const MAX_BODY: usize = 8 * 1024 * 1024;
/// What a browser may send up a relayed stream: Binance takes five messages a
/// second on a connection, and an address that keeps sending more is banned —
/// this server's, and with it every chart and recorder. Past this they wait.
const UP_PER_SECOND: usize = 5;
/// A subscribe frame is a few kilobytes at most.
const MAX_UP_MESSAGE: usize = 64 * 1024;

/// The public market streams, by venue or — where an exchange splits them over
/// several sockets — by socket.
fn stream_url(venue: &str) -> Option<&'static str> {
    Some(match venue {
        "binance-spot" => "wss://stream.binance.com:9443/ws",
        "binance-perp" => "wss://fstream.binance.com/market/ws",
        // The futures order book is on `public`, not `market`.
        "binance-perp-book" => "wss://fstream.binance.com/public/ws",
        "bybit-spot" => "wss://stream.bybit.com/v5/public/spot",
        "bybit-perp" => "wss://stream.bybit.com/v5/public/linear",
        "okx-business" => "wss://ws.okx.com/ws/v5/business",
        "okx-public" => "wss://ws.okx.com/ws/v5/public",
        "bitget" => "wss://ws.bitget.com/v2/ws/public",
        "hyperliquid" => "wss://api.hyperliquid.xyz/ws",
        "kraken-spot" => "wss://ws.kraken.com/v2",
        "kraken-perp" => "wss://futures.kraken.com/ws/v1",
        "lighter" => "wss://mainnet.zklighter.elliot.ai/stream",
        _ => return None,
    })
}

pub struct Relay {
    pub doh: Arc<Doh>,
    http: reqwest::Client,
    /// Streams relayed at once; past this, a browser keeps retrying its own route.
    streams: Semaphore,
}

impl Relay {
    pub fn new(doh: Arc<Doh>) -> anyhow::Result<Arc<Self>> {
        // A market data call answers; it never sends the server on elsewhere.
        let http = reqwest::Client::builder()
            .dns_resolver(doh.resolver())
            .user_agent(concat!("tradoor/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Arc::new(Self { doh, http, streams: Semaphore::new(500) }))
    }
}

/// Whether `url` is one of the REST calls relayed.
pub fn allowed_rest(url: &reqwest::Url) -> bool {
    // The URL is parsed, so `..` is already resolved out of the path a prefix is matched against.
    url.scheme() == "https" && url.port().is_none() && REST.iter().any(|(host, paths)| url.host_str() == Some(*host) && paths.iter().any(|p| if p.ends_with('/') { url.path().starts_with(p) && url.path().len() > p.len() } else { url.path() == *p }))
}

#[derive(Deserialize)]
pub struct RestQuery {
    url: String,
}

/// `GET /api/relay/rest?url=…`: the exchange's answer, status and body as they came.
pub async fn rest(State(state): State<AppState>, Query(q): Query<RestQuery>) -> Result<Response, ApiError> {
    let url = reqwest::Url::parse(&q.url).map_err(|_| ApiError::bad_request("not a URL"))?;
    if !allowed_rest(&url) {
        return Err(ApiError(StatusCode::FORBIDDEN, "only the exchanges' market data calls are relayed".into()));
    }
    // Everyone's relayed calls to one exchange together stay well under what it would ban this server for.
    if !state.limits.take(limits::RELAY_HOST, url.host_str().unwrap_or_default()) {
        return Ok(limits::too_many("relayed calls to this exchange"));
    }
    let upstream = state.relay.http.get(url).send().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("the exchange could not be reached: {e}")))?;
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = upstream.bytes().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("the exchange's answer broke off: {e}")))?;
    if bytes.len() > MAX_BODY {
        return Err(ApiError(StatusCode::BAD_GATEWAY, "the exchange's answer was too large to relay".into()));
    }
    let mut res = Response::new(Body::from(bytes));
    *res.status_mut() = status;
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(ct) = content_type.and_then(|ct| HeaderValue::from_str(&ct).ok()) {
        headers.insert(header::CONTENT_TYPE, ct);
    }
    Ok(res)
}

#[derive(Deserialize)]
pub struct StreamQuery {
    venue: String,
}

/// `GET /api/relay/ws?venue=binance-perp`: the venue's public stream, frame for frame.
pub async fn ws(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<StreamQuery>, upgrade: WebSocketUpgrade) -> Response {
    let Some(url) = stream_url(&q.venue) else {
        return ApiError::bad_request("an unknown venue").into_response();
    };
    // Everyone's relayed connections to one exchange together, well inside what it bans an address for.
    if !state.limits.take(limits::RELAY_HOST_OPENS, url) {
        return limits::too_many("relayed streams opened to this exchange");
    }
    // One address may hold a page's worth of streams, not the server's all.
    let held = match limits::client(&headers) {
        Some(ip) => match state.limits.hold("relay-ws", &ip, limits::RELAY_STREAMS) {
            Some(held) => Some(held),
            None => return limits::too_many("relayed streams open"),
        },
        None => None,
    };
    let relay = state.relay.clone();
    upgrade.max_message_size(MAX_UP_MESSAGE).max_frame_size(MAX_UP_MESSAGE).on_upgrade(move |client| async move {
        let _held = held;
        let Ok(_permit) = relay.streams.try_acquire() else { return };
        pipe(client, &relay.doh, url).await;
    })
}

/// Frames both ways until either side closes. The browser's hub subscribes and
/// pings as it would directly; this only carries.
async fn pipe(client: WebSocket, doh: &Doh, url: &str) {
    let upstream = match doh.connect_ws(url).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(url, error = %e, "relayed stream could not connect");
            return;
        }
    };
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut client_tx, mut client_rx) = client.split();
    let mut pace = Pace::default();
    loop {
        tokio::select! {
            frame = client_rx.next() => match frame {
                Some(Ok(ClientMessage::Text(text))) => {
                    tokio::time::sleep(pace.wait(Instant::now())).await;
                    if up_tx.send(UpstreamMessage::text(text.as_str())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(ClientMessage::Binary(bytes))) => {
                    tokio::time::sleep(pace.wait(Instant::now())).await;
                    if up_tx.send(UpstreamMessage::binary(bytes)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(ClientMessage::Ping(_) | ClientMessage::Pong(_))) => {}
                _ => break,
            },
            frame = up_rx.next() => match frame {
                Some(Ok(UpstreamMessage::Text(text))) => {
                    if client_tx.send(ClientMessage::text(text.as_str())).await.is_err() {
                        break;
                    }
                }
                Some(Ok(UpstreamMessage::Binary(bytes))) => {
                    if client_tx.send(ClientMessage::binary(bytes)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(UpstreamMessage::Ping(_) | UpstreamMessage::Pong(_) | UpstreamMessage::Frame(_))) => {}
                _ => break,
            },
        }
    }
    let _ = up_tx.close().await;
    let _ = client_tx.close().await;
}

/// When the next message up may go: at most `UP_PER_SECOND` in any second.
#[derive(Default)]
struct Pace {
    sent: VecDeque<Instant>,
}

impl Pace {
    /// How long to wait before sending one more, noting it as sent then.
    fn wait(&mut self, now: Instant) -> Duration {
        while self.sent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) {
            self.sent.pop_front();
        }
        let at = if self.sent.len() < UP_PER_SECOND { now } else { self.sent.pop_front().map_or(now, |t| t + Duration::from_secs(1)) };
        self.sent.push_back(at);
        at.saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_stream_sends_up_at_most_five_messages_a_second() {
        let mut pace = Pace::default();
        let t0 = Instant::now();
        assert!((0..5).all(|_| pace.wait(t0).is_zero()), "a page's first frames go at once");
        assert_eq!(pace.wait(t0), Duration::from_secs(1), "the sixth waits for the first to be a second old");
        assert_eq!(pace.wait(t0), Duration::from_secs(1));
        let later = t0 + Duration::from_secs(3);
        assert!(pace.wait(later).is_zero(), "a quiet stream sends again at once");
        let mut flood = Pace::default();
        let waits: Vec<Duration> = (0..20).map(|_| flood.wait(t0)).collect();
        assert_eq!(waits[19], Duration::from_secs(3), "twenty at once go five a second: the last three seconds on");
    }

    #[test]
    fn only_market_data_calls_are_relayed() {
        let ok = |u: &str| allowed_rest(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://fapi.binance.com/fapi/v1/klines?symbol=BTCUSDT&interval=1m&limit=1000"));
        assert!(ok("https://api.bybit.com/v5/market/kline?category=linear&symbol=BTCUSDT&interval=1"));
        assert!(!ok("https://fapi.binance.com/fapi/v1/order"), "no trading endpoints");
        assert!(!ok("https://evil.example/fapi/v1/klines"), "no other hosts");
        assert!(!ok("http://fapi.binance.com/fapi/v1/klines"), "https only");
        assert!(!ok("https://fapi.binance.com:8443/fapi/v1/klines"), "no other ports");
        assert!(!ok("https://fapi.binance.com/fapi/v1/klines/../order"), "the path must match exactly");
        assert!(ok("https://futures.kraken.com/api/charts/v1/trade/PF_XBTUSD/1h?from=1"));
        assert!(!ok("https://futures.kraken.com/api/charts/v1/trade/../../derivatives/api/v3/sendorder"), "a prefix after `..` is resolved");
        assert!(ok("https://www.okx.com/api/v5/market/history-candles?instId=BTC-USDT&bar=1H"));
        assert!(!ok("https://www.okx.com/api/v5/trade/order"));
        assert!(ok("https://fapi.binance.com/fapi/v1/depth?symbol=BTCUSDT&limit=1000"));
        assert!(ok("https://www.okx.com/api/v5/public/instruments?instType=SWAP&instId=BTC-USDT-SWAP"));
        assert_eq!(stream_url("okx-business"), Some("wss://ws.okx.com/ws/v5/business"));
        assert_eq!(stream_url("bybit-perp"), Some("wss://stream.bybit.com/v5/public/linear"));
        assert_eq!(stream_url("binance-perp-book"), Some("wss://fstream.binance.com/public/ws"));
        assert_eq!(stream_url("wss://anything"), None);
    }
}
