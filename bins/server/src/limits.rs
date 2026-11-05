//! How much one visitor may ask of this server, and how much everyone together
//! may ask of an exchange through it.
//!
//! Anonymous and unauthenticated by design, the API is open to anyone who can
//! reach it — so what keeps one visitor from spoiling it for the rest is here:
//! a request rate per address, a cap on long-lived streams per address, and a
//! rate per exchange host for the relay. Past the relay's, an exchange would
//! ban this server's address, and with it the recorders every chart reads.
//!
//! The address is Cloudflare's `CF-Connecting-IP` or nginx's `X-Real-IP` (see
//! `client`): the API listens on loopback only, behind nginx, which sets them. Where it is missing —
//! development, or an nginx location that does not set it — the per-address
//! limits do not apply (every visitor would share one address), and only the
//! shared ones do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::api::AppState;

/// A token bucket: `burst` requests at once, refilled at `per_min` a minute.
#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub name: &'static str,
    pub per_min: f64,
    pub burst: f64,
}

/// Every API call from one address.
pub const API: Rate = Rate { name: "api", per_min: 1200.0, burst: 300.0 };
/// Creating, changing and deleting drawings and layouts.
pub const WRITES: Rate = Rate { name: "writes", per_min: 120.0, burst: 60.0 };
/// A browser id's first drawing or layout: ids are free to make, rows are not.
pub const NEW_OWNERS: Rate = Rate { name: "new-owners", per_min: 0.5, burst: 20.0 };
/// Open interest a chart polls, which may start a coin recording.
pub const OPEN_INTEREST: Rate = Rate { name: "open-interest", per_min: 120.0, burst: 60.0 };
/// Relayed exchange REST calls from one address.
pub const RELAY_REST: Rate = Rate { name: "relay-rest", per_min: 240.0, burst: 120.0 };
/// Relayed calls to one exchange host from everyone: well inside the lightest
/// exchange limit (Binance's 2,400 weight a minute; a kline page weighs 2 to 10).
pub const RELAY_HOST: Rate = Rate { name: "relay-host", per_min: 300.0, burst: 150.0 };
/// Relayed candle calls to Bitfinex from everyone: it answers no browser itself,
/// so every chart on it comes through here, and it allows an address thirty a minute.
pub const RELAY_BITFINEX: Rate = Rate { name: "relay-bitfinex", per_min: 25.0, burst: 15.0 };
/// Relayed streams opened from one address.
pub const RELAY_OPENS: Rate = Rate { name: "relay-opens", per_min: 60.0, burst: 30.0 };
/// Relayed streams opened to one exchange socket from everyone: Binance bans an
/// address that opens 300 in five minutes.
pub const RELAY_HOST_OPENS: Rate = Rate { name: "relay-host-opens", per_min: 40.0, burst: 40.0 };
/// Relayed streams one address keeps open: a page opens one per exchange socket.
pub const RELAY_STREAMS: usize = 16;
/// Live liquidation streams one address keeps open: one per chart.
pub const LIVE_STREAMS: usize = 8;

/// Buckets idle this long are full again, and forgotten.
const IDLE_SECS: f64 = 600.0;
const PRUNE_AT: usize = 20_000;

struct Bucket {
    tokens: f64,
    at: Instant,
}

#[derive(Default)]
pub struct Limits {
    buckets: Mutex<HashMap<(&'static str, String), Bucket>>,
    open: Arc<Mutex<HashMap<(&'static str, String), usize>>>,
}

/// One of a capped kind of stream, held open; dropping it frees the place.
pub struct Held {
    open: Arc<Mutex<HashMap<(&'static str, String), usize>>>,
    key: (&'static str, String),
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Ok(mut open) = self.open.lock() {
            if let Some(n) = open.get_mut(&self.key) {
                *n -= 1;
                if *n == 0 {
                    open.remove(&self.key);
                }
            }
        }
    }
}

impl Limits {
    /// Takes one request from `key`'s bucket; false when it is empty.
    pub fn take(&self, rate: Rate, key: &str) -> bool {
        self.take_at(rate, key, Instant::now())
    }

    fn take_at(&self, rate: Rate, key: &str, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else { return true };
        if buckets.len() > PRUNE_AT {
            buckets.retain(|_, b| now.duration_since(b.at).as_secs_f64() < IDLE_SECS);
        }
        let b = buckets.entry((rate.name, key.to_string())).or_insert(Bucket { tokens: rate.burst, at: now });
        let elapsed = now.duration_since(b.at).as_secs_f64();
        b.tokens = (b.tokens + elapsed * rate.per_min / 60.0).min(rate.burst);
        b.at = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// A place among `max` open streams of `kind` for `key`, or None when all are taken.
    pub fn hold(&self, kind: &'static str, key: &str, max: usize) -> Option<Held> {
        let mut open = self.open.lock().ok()?;
        let n = open.entry((kind, key.to_string())).or_insert(0);
        if *n >= max {
            return None;
        }
        *n += 1;
        Some(Held { open: self.open.clone(), key: (kind, key.to_string()) })
    }
}

/// Whether the address may start one more browser id's rows (see `NEW_OWNERS`).
pub fn new_owner(state: &AppState, headers: &HeaderMap) -> bool {
    client(headers).is_none_or(|ip| state.limits.take(NEW_OWNERS, &ip))
}

/// The visitor's address: Cloudflare's `CF-Connecting-IP` where the site sits
/// behind Cloudflare — nginx's own address for the visitor is then Cloudflare's
/// — else nginx's `X-Real-IP`. Cloudflare's is only as good as the host's rule
/// that nothing but Cloudflare reaches nginx (deploy/nginx/http.conf).
pub fn client(headers: &HeaderMap) -> Option<String> {
    ["cf-connecting-ip", "x-real-ip"].iter().find_map(|name| {
        let ip = headers.get(*name)?.to_str().ok()?.trim();
        (!ip.is_empty() && ip.len() <= 64).then(|| ip.to_string())
    })
}

/// `429 Too Many Requests`, with how long to wait.
pub fn too_many(what: &str) -> Response {
    let mut res = (StatusCode::TOO_MANY_REQUESTS, axum::Json(json!({ "error": format!("too many {what}; wait a moment") }))).into_response();
    res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
    res
}

/// Every API request passes here: the address's overall rate, and the rate of
/// the kind of request it is.
pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(ip) = client(req.headers()) {
        let path = req.uri().path();
        let rate = if path.starts_with("/relay/rest") {
            Some(RELAY_REST)
        } else if path.starts_with("/relay/ws") {
            Some(RELAY_OPENS)
        } else if path.starts_with("/open-interest") {
            Some(OPEN_INTEREST)
        } else if matches!(*req.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE) {
            Some(WRITES)
        } else {
            None
        };
        if !state.limits.take(API, &ip) {
            return too_many("requests");
        }
        if let Some(rate) = rate {
            if !state.limits.take(rate, &ip) {
                return too_many(rate.name);
            }
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_bucket_allows_its_burst_then_its_rate() {
        let limits = Limits::default();
        let rate = Rate { name: "t", per_min: 60.0, burst: 3.0 };
        let t0 = Instant::now();
        assert!((0..3).all(|_| limits.take_at(rate, "a", t0)));
        assert!(!limits.take_at(rate, "a", t0), "the burst is spent");
        assert!(limits.take_at(rate, "b", t0), "another address has its own");
        assert!(limits.take_at(rate, "a", t0 + Duration::from_secs(1)), "a second refills one");
        assert!(!limits.take_at(rate, "a", t0 + Duration::from_secs(1)));
    }

    #[test]
    fn held_places_come_back_when_dropped() {
        let limits = Limits::default();
        let a = limits.hold("s", "ip", 2).unwrap();
        let _b = limits.hold("s", "ip", 2).unwrap();
        assert!(limits.hold("s", "ip", 2).is_none());
        assert!(limits.hold("s", "other", 2).is_some());
        drop(a);
        assert!(limits.hold("s", "ip", 2).is_some());
    }

    #[test]
    fn the_address_is_nginx_s() {
        let mut h = HeaderMap::new();
        assert_eq!(client(&h), None);
        h.insert("x-real-ip", HeaderValue::from_static("203.0.113.9"));
        assert_eq!(client(&h).as_deref(), Some("203.0.113.9"));
        // Behind Cloudflare, nginx's address is Cloudflare's; the visitor's is in its header.
        h.insert("cf-connecting-ip", HeaderValue::from_static("198.51.100.4"));
        assert_eq!(client(&h).as_deref(), Some("198.51.100.4"));
    }
}
