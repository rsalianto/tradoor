//! Who has tradoor on screen, and how the server is doing, for its owner alone.
//!
//! A tab says it is here once a minute while it is shown (`POST /api/presence`):
//! only its browser id and when, held in memory, and forgotten after a day or
//! on a restart. `GET /api/admin/status` answers with counts and nothing else —
//! never an id, a name or a Telegram user — and only to the key set as
//! `TRADOOR_ADMIN_KEY`. Without one it is not there at all. It only reads.
//!
//! The key goes in the `Authorization` header, never the address, so no log
//! holds it; it is compared in time that does not depend on how much of it was
//! right; and wrong ones are refused past a few a minute from one address, and
//! from everyone together.
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::api::{owner, ApiResult, AppState};
use crate::limits::{self, Rate};

/// On screen within this long counts as here now.
const ONLINE_MS: i64 = 2 * 60_000;
const RECENT_MS: i64 = 15 * 60_000;
const DAY_MS: i64 = 86_400_000;
/// Browsers held at most: past this, new ones are not counted rather than held.
const MAX_BROWSERS: usize = 50_000;
/// How often the peak is read and the day's oldest let go.
const TICK: Duration = Duration::from_secs(30);
/// A key shorter than this is refused, and the page stays off.
const MIN_KEY: usize = 32;
/// Wrong keys from one address, and from every address together.
const WRONG_KEY: Rate = Rate { name: "admin-wrong", per_min: 3.0, burst: 10.0 };
const WRONG_KEYS: Rate = Rate { name: "admin-wrong-all", per_min: 20.0, burst: 40.0 };

pub struct Presence {
    seen: Mutex<HashMap<String, i64>>,
    /// The UTC day, the most here at once in it, and when.
    peak: Mutex<(i64, usize, i64)>,
    key: Option<String>,
    started: i64,
    up: Instant,
    /// How many are here now, as last worked out: what every page is told.
    online: AtomicUsize,
}

/// How many are here, worked out at `now`.
#[derive(Debug, PartialEq)]
struct Counts {
    online: usize,
    recent: usize,
    today: usize,
    online_ids: Vec<String>,
}

impl Presence {
    pub fn start(key: &str) -> Arc<Self> {
        let key = key.trim();
        let key = match key.len() {
            0 => None,
            n if n < MIN_KEY => {
                tracing::warn!("TRADOOR_ADMIN_KEY is shorter than {MIN_KEY} characters: the status page stays off");
                None
            }
            _ => Some(key.to_string()),
        };
        let presence = Arc::new(Self { seen: Mutex::new(HashMap::new()), peak: Mutex::new((0, 0, 0)), key, started: now_ms(), up: Instant::now(), online: AtomicUsize::new(0) });
        let ticking = presence.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(TICK);
            loop {
                every.tick().await;
                ticking.tick(now_ms());
            }
        });
        presence
    }

    fn here(&self, id: &str, now: i64) {
        let Ok(mut seen) = self.seen.lock() else { return };
        if seen.len() < MAX_BROWSERS || seen.contains_key(id) {
            seen.insert(id.to_string(), now);
        }
    }

    fn counts(&self, now: i64) -> Counts {
        let Ok(seen) = self.seen.lock() else { return Counts { online: 0, recent: 0, today: 0, online_ids: Vec::new() } };
        let day = now - now.rem_euclid(DAY_MS);
        let online_ids: Vec<String> = seen.iter().filter(|(_, at)| now - **at < ONLINE_MS).map(|(id, _)| id.clone()).collect();
        Counts { online: online_ids.len(), recent: seen.values().filter(|at| now - **at < RECENT_MS).count(), today: seen.values().filter(|at| **at >= day).count(), online_ids }
    }

    /// How many have tradoor on screen now, worked out at the last tick: a number, never who.
    pub fn online(&self) -> usize {
        self.online.load(Ordering::Relaxed)
    }

    /// The peak so far today, and the browsers not seen for a day let go.
    fn tick(&self, now: i64) {
        let online = self.counts(now).online;
        self.online.store(online, Ordering::Relaxed);
        if let Ok(mut peak) = self.peak.lock() {
            let day = now.div_euclid(DAY_MS);
            if peak.0 != day {
                *peak = (day, 0, now);
            }
            if online > peak.1 {
                (peak.1, peak.2) = (online, now);
            }
        }
        if let Ok(mut seen) = self.seen.lock() {
            seen.retain(|_, at| now - *at < DAY_MS);
        }
    }

    /// Whether `headers` carry the key: false when there is none to carry.
    fn admits(&self, headers: &HeaderMap) -> bool {
        let Some(key) = &self.key else { return false };
        let given = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
        same(given.as_bytes(), key.as_bytes())
    }
}

/// Equal, in time that depends on the length alone.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn now_ms() -> i64 {
    crate::liquidations::now_ms()
}

/// `POST /api/presence`: this browser has tradoor on screen. Answered with how
/// many have it on screen now, for the count beside the messenger.
pub async fn ping(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let id = owner(&headers)?;
    state.presence.here(&id, now_ms());
    Ok(Json(json!({ "online": state.presence.online().max(1) })))
}

/// The answer for a request without the admin key; none for one with it.
/// No key set is no page at all; a wrong one is refused, and past a few a minute not even read.
pub fn refused(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let presence = &state.presence;
    if presence.key.is_none() {
        return Some(StatusCode::NOT_FOUND.into_response());
    }
    if presence.admits(headers) {
        return None;
    }
    let ip = limits::client(headers);
    let allowed = ip.as_deref().is_none_or(|ip| state.limits.take(WRONG_KEY, ip)) && state.limits.take(WRONG_KEYS, "all");
    if !allowed {
        return Some(limits::too_many("wrong keys"));
    }
    tracing::warn!(from = ip.as_deref().unwrap_or("?"), "a wrong admin key");
    Some((StatusCode::UNAUTHORIZED, Json(json!({ "error": "wrong key" }))).into_response())
}

/// `GET /api/admin/status`: the counts and the server's state, for the key's holder.
pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let presence = &state.presence;
    let now = now_ms();
    let counts = presence.counts(now);
    let signed_in = counts.online_ids.iter().filter(|id| state.sessions.is_account(id)).count();
    let peak = presence.peak.lock().map(|p| if p.0 == now.div_euclid(DAY_MS) { (p.1.max(counts.online), p.2) } else { (counts.online, now) }).unwrap_or((counts.online, now));
    let (db_ok, db_ms, telegram) = telegram_counts(&state, now).await;
    let scripts = crate::scripts::counts(&state, now).await;
    let pool = |p: &deadpool_postgres::Pool| {
        let s = p.status();
        json!({ "max": s.max_size, "open": s.size, "inUse": s.size.saturating_sub(s.available), "waiting": s.waiting })
    };
    let body = json!({
        "now": now,
        "users": { "online": counts.online, "signedIn": signed_in, "last15m": counts.recent, "today": counts.today, "peakToday": peak.0, "peakAt": peak.1 },
        "telegram": telegram,
        "scripts": scripts,
        "server": {
            "version": env!("CARGO_PKG_VERSION"),
            "startedAt": presence.started,
            "upSecs": presence.up.elapsed().as_secs(),
            "db": { "ok": db_ok, "ms": db_ms },
            "requests": pool(&state.pool),
            "work": pool(&state.work),
            "memoryMb": memory_mb(),
            "load": load(),
        },
    });
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

/// Accounts signed in with Telegram and chats linked for alerts, with how long the database took.
async fn telegram_counts(state: &AppState, now: i64) -> (bool, Option<f64>, Value) {
    let started = Instant::now();
    let day = now - now.rem_euclid(DAY_MS);
    let read = async {
        let client = state.pool.get().await?;
        let row = client
            .query_one(
                "SELECT (SELECT count(*) FROM account), (SELECT count(*) FROM account WHERE since >= $1),
                        (SELECT count(*) FROM account WHERE seen_at >= $2), (SELECT count(DISTINCT chat_id) FROM telegram_chat)",
                &[&day, &(now - DAY_MS)],
            )
            .await?;
        anyhow::Ok((row.get::<_, i64>(0), row.get::<_, i64>(1), row.get::<_, i64>(2), row.get::<_, i64>(3)))
    };
    match read.await {
        Ok((accounts, today, seen, chats)) => (
            true,
            Some((started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0),
            json!({ "bot": state.telegram.is_some(), "accounts": accounts, "newToday": today, "seen24h": seen, "alertChats": chats }),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "admin status: the database");
            (false, None, json!({ "bot": state.telegram.is_some() }))
        }
    }
}

/// This process's resident memory, from Linux's own account of it.
fn memory_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some((kb / 102.4).round() / 10.0)
}

/// The host's load over one, five and fifteen minutes.
fn load() -> Option<[f64; 3]> {
    let raw = std::fs::read_to_string("/proc/loadavg").ok()?;
    let mut parts = raw.split_whitespace().map(|p| p.parse::<f64>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(key: Option<&str>) -> Presence {
        Presence { seen: Mutex::new(HashMap::new()), peak: Mutex::new((0, 0, 0)), key: key.map(str::to_string), started: 0, up: Instant::now(), online: AtomicUsize::new(0) }
    }

    #[test]
    fn a_browser_on_screen_counts_as_here_for_two_minutes_and_today_for_the_day() {
        let p = presence(None);
        let now = 20_000 * DAY_MS + 12 * 3_600_000;
        p.here("a", now - 60_000);
        p.here("b", now - 5 * 60_000);
        p.here("c", now - 11 * 3_600_000);
        p.here("d", now - 13 * 3_600_000);
        let c = p.counts(now);
        assert_eq!((c.online, c.recent, c.today), (1, 2, 3), "d was seen yesterday, UTC");
        p.here("a", now);
        assert_eq!(p.counts(now).online, 1, "one browser once, however often it says so");

        p.tick(now);
        assert_eq!(p.peak.lock().unwrap().1, 1);
        p.tick(now + DAY_MS + 1);
        assert!(p.seen.lock().unwrap().is_empty(), "a day without a word lets a browser go");
        assert_eq!(p.peak.lock().unwrap().1, 0, "a new day starts its own peak");
    }

    #[test]
    fn past_its_size_new_browsers_are_not_held() {
        let p = presence(None);
        for i in 0..MAX_BROWSERS {
            p.here(&format!("id{i}"), 1);
        }
        p.here("one-more", 2);
        p.here("id0", 3);
        let seen = p.seen.lock().unwrap();
        assert_eq!(seen.len(), MAX_BROWSERS);
        assert_eq!(seen.get("id0"), Some(&3), "one already held still counts");
    }

    #[test]
    fn only_the_whole_key_in_the_authorization_header_admits() {
        let key = "k".repeat(40);
        let with = |value: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::AUTHORIZATION, value.parse().unwrap());
            h
        };
        let p = presence(Some(&key));
        assert!(p.admits(&with(&format!("Bearer {key}"))));
        assert!(!p.admits(&with(&format!("Bearer {}", &key[..39]))));
        assert!(!p.admits(&with(&format!("Bearer {key}x"))));
        assert!(!p.admits(&with(&key)), "without the scheme");
        assert!(!p.admits(&HeaderMap::new()));
        assert!(!presence(None).admits(&with("Bearer ")), "no key set admits nobody, an empty one included");
    }

    #[tokio::test]
    async fn a_short_key_leaves_the_page_off() {
        assert!(Presence::start("short").key.is_none());
        assert!(Presence::start("").key.is_none());
        assert!(Presence::start(&"x".repeat(MIN_KEY)).key.is_some());
    }
}
