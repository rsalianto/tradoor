//! Frost Scripts: the indicators accounts write, kept on the server so they follow
//! their author to every device. Only an account writes one — a browser that has
//! not signed in with Telegram reads the examples and nothing more.
//!
//! A script is kept as its text and checked here by the formula crate itself
//! (crates/frost-script), the one the browser runs it with: whether it checks is kept
//! beside it. Nothing of it runs on the server. The moderator, holding the admin
//! key, lists every script and can remove one — kept, with why, but run for no
//! one — restore it, or delete it.
//!
//! A script is its author's alone until they make it public: then it is listed in
//! the Community, under the account's username (crate::username), for any reader to add
//! to a chart — signed in or not. Made private again, or removed, it runs for its
//! author alone (removed: for no one).
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tradoor_store::now_ms;

use crate::api::{owner, ApiError, ApiResult, AppState};
use crate::presence::refused;

/// Scripts one account keeps.
const MAX_PER_OWNER: i64 = 100;
const MAX_NAME: usize = 40;
const MAX_WHY: usize = 200;
/// Scripts the moderator's list shows at once, newest first.
const MAX_LISTED: i64 = 300;
const DAY_MS: i64 = 86_400_000;
/// Public scripts the Community lists at once, the last changed first.
const MAX_COMMUNITY: i64 = 500;
/// A description, in characters: at most, and at least to share the script.
const MAX_ABOUT: usize = 1000;
const SHARED_ABOUT: usize = 30;
/// Of a description, what the Community's list carries; the rest comes with the script's text.
const LISTED_ABOUT: i32 = 240;

fn account(state: &AppState, headers: &HeaderMap) -> ApiResult<String> {
    let owner = owner(headers)?;
    if state.sessions.is_account(&owner) {
        Ok(owner)
    } else {
        Err(ApiError::forbidden("sign in with Telegram to write Frost Scripts"))
    }
}

#[derive(Deserialize)]
pub struct Body {
    name: String,
    source: String,
    /// Listed in the Community; left out, a new one is private and a changed one stays as it was.
    public: Option<bool>,
    /// What it shows and how to read it; left out, it stays as it was.
    about: Option<String>,
    /// The chart it is shown on in the showcase; left out, it stays as it was.
    preview: Option<Preview>,
}

#[derive(Deserialize)]
pub struct Preview {
    market: String,
    timeframe: String,
    #[serde(default)]
    inputs: Vec<f64>,
}

/// A preview as it is kept, or nothing where it is not one: a market's id, a timeframe, and at most a script's settings.
fn previewed(p: &Preview) -> Option<(String, String, String)> {
    let (venue, symbol) = p.market.split_once(':')?;
    let market_ok = p.market.len() <= 64
        && !venue.is_empty()
        && venue.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !symbol.is_empty()
        && symbol.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b));
    let tf_ok = (2..=4).contains(&p.timeframe.len()) && p.timeframe.bytes().all(|b| b.is_ascii_alphanumeric());
    let inputs_ok = p.inputs.len() <= tradoor_frost_script::MAX_INPUTS && p.inputs.iter().all(|v| v.is_finite());
    (market_ok && tf_ok && inputs_ok).then(|| (p.market.clone(), p.timeframe.clone(), p.inputs.iter().map(f64::to_string).collect::<Vec<_>>().join(":")))
}

/// The preview of a row's three columns, as the browser reads it.
fn preview_json(market: Option<String>, timeframe: Option<String>, inputs: Option<String>) -> Value {
    match (market, timeframe) {
        (Some(market), Some(timeframe)) => {
            let inputs: Vec<f64> = inputs.unwrap_or_default().split(':').filter_map(|v| v.parse().ok()).collect();
            json!({ "market": market, "timeframe": timeframe, "inputs": inputs })
        }
        _ => Value::Null,
    }
}

/// Whether a script checks, worked out off the threads that answer requests, on one
/// of its own with room to spare: how deep a script may go is bounded
/// (`tradoor_frost_script::MAX_LEVEL`), and this is the margin over it. Four at a time.
async fn checks(source: &str) -> bool {
    static AT_ONCE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let Ok(_turn) = AT_ONCE.acquire().await else { return false };
    let source = source.to_string();
    tokio::task::spawn_blocking(move || {
        std::thread::Builder::new().name("frost-check".into()).stack_size(64 << 20).spawn(move || tradoor_frost_script::describe(&source).is_ok()).ok().and_then(|t| t.join().ok()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// The name trimmed, and both held to their limits.
fn checked(body: &Body) -> ApiResult<(String, Option<String>)> {
    let name: String = body.name.trim().chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(format!("a name is 1 to {MAX_NAME} characters")));
    }
    if body.source.len() > tradoor_frost_script::MAX_SOURCE {
        return Err(ApiError::bad_request(format!("a script is {} characters at most", tradoor_frost_script::MAX_SOURCE)));
    }
    let about = body.about.as_deref().map(described);
    if about.as_ref().is_some_and(|a| a.chars().count() > MAX_ABOUT) {
        return Err(ApiError::bad_request(format!("a description is {MAX_ABOUT} characters at most")));
    }
    Ok((name, about))
}

/// A description as it is kept: plain lines, trimmed, no more than one blank line between paragraphs.
fn described(raw: &str) -> String {
    let lines: Vec<String> = raw.lines().map(|l| l.replace('\t', " ").chars().filter(|c| !c.is_control()).collect::<String>().trim_end().to_string()).collect();
    let mut out: Vec<&str> = Vec::new();
    for line in &lines {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

fn new_id() -> String {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("the system's randomness");
    bytes.iter().map(|b| char::from(LETTERS[usize::from(*b) % LETTERS.len()])).collect()
}

fn valid_id(id: &str) -> bool {
    (6..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn row(r: &tokio_postgres::Row) -> Value {
    json!({
        "id": r.get::<_, String>(0),
        "name": r.get::<_, String>(1),
        "source": r.get::<_, String>(2),
        "checks": r.get::<_, bool>(3),
        "updated": r.get::<_, i64>(4),
        "removed": r.get::<_, Option<i64>>(5).is_some(),
        "why": r.get::<_, Option<String>>(6),
        "public": r.get::<_, bool>(7),
        "about": r.get::<_, String>(8),
        "preview": preview_json(r.get(9), r.get(10), r.get(11)),
    })
}

const COLUMNS: &str = "id, name, source, checks, updated_at, removed_at, removed_why, public, about, preview_market, preview_timeframe, preview_inputs";

/// Made public, or public already, a script needs a username to be listed under, a description that says what it is, and to check.
async fn may_publish(client: &deadpool_postgres::Client, owner: &str, public: bool, checks: bool, about: &str) -> ApiResult<()> {
    if !public {
        return Ok(());
    }
    if about.chars().count() < SHARED_ABOUT {
        return Err(ApiError::bad_request(format!("describe it first, in {SHARED_ABOUT} characters at least: what it shows, and how to read it")));
    }
    if crate::username::of(client, owner).await?.is_none() {
        return Err(ApiError::bad_request("choose your username first, in Settings: public scripts are shared under it"));
    }
    if !checks {
        return Err(ApiError::bad_request("put the script's mistake right first: only one that checks is made public"));
    }
    Ok(())
}

/// `GET /api/scripts`: this account's scripts, the last changed first — a removed one too, saying why — and its username.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let client = state.pool.get().await?;
    let rows = client.query(&format!("SELECT {COLUMNS} FROM frost_script WHERE owner = $1 ORDER BY updated_at DESC"), &[&owner]).await?;
    let username = crate::username::of(&client, &owner).await?;
    Ok(Json(json!({ "scripts": rows.iter().map(row).collect::<Vec<_>>(), "username": username })))
}

/// `GET /api/community`: the public scripts, the last changed first, by their authors' usernames — without their text.
pub async fn community(State(state): State<AppState>) -> ApiResult<Response> {
    let rows = state
        .pool
        .get()
        .await?
        .query(
            &format!(
                "SELECT s.id, s.name, u.name, s.updated_at, s.created_at, left(s.about, {LISTED_ABOUT}), length(s.about) > {LISTED_ABOUT}, s.preview_market, s.preview_timeframe, s.preview_inputs
                 FROM frost_script s JOIN account_username u ON u.owner = s.owner
                 WHERE s.public AND s.removed_at IS NULL AND s.checks ORDER BY s.updated_at DESC LIMIT {MAX_COMMUNITY}"
            ),
            &[],
        )
        .await?;
    let scripts: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, String>(0),
                "name": r.get::<_, String>(1),
                "username": r.get::<_, String>(2),
                "updated": r.get::<_, i64>(3),
                "created": r.get::<_, i64>(4),
                "about": r.get::<_, String>(5),
                "more": r.get::<_, bool>(6),
                "preview": preview_json(r.get(7), r.get(8), r.get(9)),
            })
        })
        .collect();
    Ok(([(header::CACHE_CONTROL, "public, max-age=60")], Json(json!({ "scripts": scripts }))).into_response())
}

/// `GET /api/community/{id}`: one public script, with its text, to draw on any chart.
pub async fn community_script(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Response> {
    if !valid_id(&id) {
        return Err(ApiError::bad_request("not a script's id"));
    }
    let r = state
        .pool
        .get()
        .await?
        .query_opt(
            "SELECT s.id, s.name, s.source, u.name, s.updated_at, s.about, s.created_at, s.preview_market, s.preview_timeframe, s.preview_inputs FROM frost_script s JOIN account_username u ON u.owner = s.owner
             WHERE s.id = $1 AND s.public AND s.removed_at IS NULL",
            &[&id],
        )
        .await?;
    let Some(r) = r else { return Err(ApiError::not_found("no such public script: its author may have made it private")) };
    let body = json!({
        "id": r.get::<_, String>(0),
        "name": r.get::<_, String>(1),
        "source": r.get::<_, String>(2),
        "username": r.get::<_, String>(3),
        "updated": r.get::<_, i64>(4),
        "about": r.get::<_, String>(5),
        "created": r.get::<_, i64>(6),
        "preview": preview_json(r.get(7), r.get(8), r.get(9)),
    });
    // Asked again on every page: one made private, or changed, is so on the next.
    Ok(([(header::CACHE_CONTROL, "no-cache")], Json(body)).into_response())
}

/// `POST /api/scripts`: a new one.
pub async fn create(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Body>) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let (name, about) = checked(&body)?;
    let checks = checks(&body.source).await;
    let about = about.unwrap_or_default();
    let client = state.pool.get().await?;
    may_publish(&client, &owner, body.public == Some(true), checks, &about).await?;
    let held: i64 = client.query_one("SELECT count(*) FROM frost_script WHERE owner = $1", &[&owner]).await?.get(0);
    let preview = body.preview.as_ref().and_then(previewed);
    let (pm, pt, pi) = (preview.as_ref().map(|p| &p.0), preview.as_ref().map(|p| &p.1), preview.as_ref().map(|p| &p.2));
    if held >= MAX_PER_OWNER {
        return Err(ApiError::bad_request(format!("{MAX_PER_OWNER} scripts at most: delete one first")));
    }
    let (id, now) = (new_id(), now_ms());
    let r = client
        .query_one(
            &format!(
                "INSERT INTO frost_script (id, owner, name, source, checks, created_at, updated_at, public, about, preview_market, preview_timeframe, preview_inputs)
                 VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, $9, $10, $11) RETURNING {COLUMNS}"
            ),
            &[&id, &owner, &name, &body.source, &checks, &now, &body.public.unwrap_or(false), &about, &pm, &pt, &pi],
        )
        .await?;
    Ok(Json(row(&r)))
}

/// `PUT /api/scripts/{id}`: one of this account's, changed. A removed one stays as the moderator left it.
pub async fn update(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Body>) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let (name, about) = checked(&body)?;
    let checks = checks(&body.source).await;
    let client = state.pool.get().await?;
    // What it is to be once changed: a public one stays held to the rules for sharing, whatever is left out.
    let Some(held) = client.query_opt("SELECT public, about FROM frost_script WHERE id = $1 AND owner = $2 AND removed_at IS NULL", &[&id, &owner]).await? else {
        return Err(ApiError::not_found("no such script of yours, or the moderator removed it"));
    };
    let about = about.unwrap_or_else(|| held.get(1));
    may_publish(&client, &owner, body.public.unwrap_or_else(|| held.get(0)), checks, &about).await?;
    let preview = body.preview.as_ref().and_then(previewed);
    let (pm, pt, pi) = (preview.as_ref().map(|p| &p.0), preview.as_ref().map(|p| &p.1), preview.as_ref().map(|p| &p.2));
    let r = client
        .query_opt(
            &format!(
                "UPDATE frost_script SET name = $3, source = $4, checks = $5, updated_at = $6, public = COALESCE($7, public), about = $8,
                 preview_market = COALESCE($9, preview_market), preview_timeframe = COALESCE($10, preview_timeframe), preview_inputs = COALESCE($11, preview_inputs)
                 WHERE id = $1 AND owner = $2 AND removed_at IS NULL RETURNING {COLUMNS}"
            ),
            &[&id, &owner, &name, &body.source, &checks, &now_ms(), &body.public, &about, &pm, &pt, &pi],
        )
        .await?;
    match r {
        Some(r) => Ok(Json(row(&r))),
        None => Err(ApiError::not_found("no such script of yours, or the moderator removed it")),
    }
}

/// `DELETE /api/scripts/{id}`: one of this account's, gone.
pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let owner = account(&state, &headers)?;
    state.pool.get().await?.execute("DELETE FROM frost_script WHERE id = $1 AND owner = $2", &[&id, &owner]).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// For the status page: how many, by how many authors, and how many removed.
pub async fn counts(state: &AppState, now: i64) -> Value {
    let read = async {
        let r = state
            .pool
            .get()
            .await?
            .query_one(
                "SELECT count(*), count(DISTINCT owner), count(*) FILTER (WHERE removed_at IS NOT NULL), count(*) FILTER (WHERE created_at >= $1), count(*) FILTER (WHERE public AND removed_at IS NULL) FROM frost_script",
                &[&(now - now.rem_euclid(DAY_MS))],
            )
            .await?;
        anyhow::Ok(json!({ "total": r.get::<_, i64>(0), "authors": r.get::<_, i64>(1), "removed": r.get::<_, i64>(2), "newToday": r.get::<_, i64>(3), "public": r.get::<_, i64>(4) }))
    };
    read.await.unwrap_or(Value::Null)
}

#[derive(Deserialize)]
pub struct Listing {
    /// Removed ones alone, or none of them.
    removed: Option<bool>,
}

fn no_store(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

/// `GET /api/admin/scripts`: every script, the last changed first, with its author's
/// name — without its text, which `GET /api/admin/scripts/{id}` gives.
pub async fn moderate_list(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<Listing>) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let filter = match q.removed {
        Some(true) => "WHERE s.removed_at IS NOT NULL",
        Some(false) => "WHERE s.removed_at IS NULL",
        None => "",
    };
    let read = async {
        let rows = state
            .pool
            .get()
            .await?
            .query(
                &format!(
                    "SELECT s.id, s.name, s.checks, s.created_at, s.updated_at, s.removed_at, s.removed_why, length(s.source), a.name, s.owner, s.public, u.name, s.about
                     FROM frost_script s LEFT JOIN LATERAL (SELECT name FROM account WHERE owner = s.owner ORDER BY since LIMIT 1) a ON true
                     LEFT JOIN account_username u ON u.owner = s.owner
                     {filter} ORDER BY s.updated_at DESC LIMIT {MAX_LISTED}"
                ),
                &[],
            )
            .await?;
        anyhow::Ok(rows)
    };
    match read.await {
        Ok(rows) => no_store(json!({
            "scripts": rows.iter().map(|r| json!({
                "id": r.get::<_, String>(0),
                "name": r.get::<_, String>(1),
                "checks": r.get::<_, bool>(2),
                "created": r.get::<_, i64>(3),
                "updated": r.get::<_, i64>(4),
                "removed": r.get::<_, Option<i64>>(5).is_some(),
                "why": r.get::<_, Option<String>>(6),
                "size": r.get::<_, i32>(7),
                "author": r.get::<_, Option<String>>(8).unwrap_or_else(|| "(no name)".to_string()),
                "authorId": crate::api::short_owner(&r.get::<_, String>(9)),
                "public": r.get::<_, bool>(10),
                "username": r.get::<_, Option<String>>(11),
                "about": r.get::<_, String>(12),
            })).collect::<Vec<_>>(),
        })),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// `GET /api/admin/scripts/{id}`: one script's text, to read before acting on it.
pub async fn moderate_read(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    if !valid_id(&id) {
        return ApiError::bad_request("not a script's id").into_response();
    }
    let read = async { anyhow::Ok(state.pool.get().await?.query_opt("SELECT source FROM frost_script WHERE id = $1", &[&id]).await?) };
    match read.await {
        Ok(Some(r)) => no_store(json!({ "id": id, "source": r.get::<_, String>(0) })),
        Ok(None) => ApiError::not_found("no such script").into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
pub struct Action {
    /// `remove`, `restore` or `delete`.
    action: String,
    why: Option<String>,
}

/// `POST /api/admin/scripts/{id}`: removed (kept, run for no one), restored, or deleted for good.
pub async fn moderate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(a): Json<Action>) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    if !valid_id(&id) {
        return ApiError::bad_request("not a script's id").into_response();
    }
    let why: Option<String> = a.why.map(|w| w.trim().chars().filter(|c| !c.is_control()).take(MAX_WHY).collect::<String>()).filter(|w| !w.is_empty());
    let done = async {
        let client = state.pool.get().await?;
        let n = match a.action.as_str() {
            "remove" => client.execute("UPDATE frost_script SET removed_at = $2, removed_why = $3 WHERE id = $1", &[&id, &now_ms(), &why]).await?,
            "restore" => client.execute("UPDATE frost_script SET removed_at = NULL, removed_why = NULL WHERE id = $1", &[&id]).await?,
            "delete" => client.execute("DELETE FROM frost_script WHERE id = $1", &[&id]).await?,
            _ => return Ok(None),
        };
        anyhow::Ok(Some(n))
    };
    match done.await {
        Ok(Some(0)) => ApiError::not_found("no such script").into_response(),
        Ok(Some(_)) => {
            tracing::info!(script = %id, action = %a.action, "moderated");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => ApiError::bad_request("remove, restore or delete").into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_script_is_checked_off_the_request_threads() {
        assert!(checks("plot(close, \"C\")").await);
        assert!(!checks("x = ema(close)").await, "kept, and said not to check");
        // One that nests as deep as the language allows is answered, not the end of the server.
        let mut deep = String::from("fn g0(x) {\n  return x + 1\n}\n");
        for i in 1..64 {
            deep += &format!("fn g{i}(x) {{\n  return {}g{}(x){}\n}}\n", "(".repeat(30), i - 1, " + 1)".repeat(30));
        }
        assert!(!checks(&(deep + "plot(g63(close), \"G\")")).await);
    }

    #[test]
    fn a_name_and_a_text_are_held_to_their_limits() {
        let ok = |name: &str, source: &str| checked(&Body { name: name.into(), source: source.into(), public: None, about: None, preview: None });
        assert_eq!(ok("  My RSI ", "plot(close, \"C\")").unwrap(), ("My RSI".to_string(), None));
        assert!(ok("", "plot(close)").is_err());
        assert!(ok(&"n".repeat(41), "plot(close)").is_err());
        assert!(ok("Big", &"x".repeat(tradoor_frost_script::MAX_SOURCE + 1)).is_err());
        assert_eq!(ok("a\u{7}b", "").unwrap().0, "ab", "no control characters in a name");
    }

    #[test]
    fn a_description_is_plain_lines_and_held_to_its_length() {
        assert_eq!(described("  An EMA.\n\n\n\nCrosses\tmarked.  \n\n"), "An EMA.\n\nCrosses marked.");
        assert_eq!(described("a\r\nb\u{7}c"), "a\nbc");
        let long = |n: usize| checked(&Body { name: "X".into(), source: String::new(), public: None, about: Some("x".repeat(n)), preview: None });
        assert!(long(MAX_ABOUT).is_ok() && long(MAX_ABOUT + 1).is_err());
    }

    #[test]
    fn a_preview_is_a_market_a_timeframe_and_settings() {
        let p = |market: &str, timeframe: &str, inputs: Vec<f64>| previewed(&Preview { market: market.into(), timeframe: timeframe.into(), inputs });
        assert_eq!(p("binance-perp:BTCUSDT", "1h", vec![20.0, 2.5]), Some(("binance-perp:BTCUSDT".into(), "1h".into(), "20:2.5".into())));
        assert!(p("hyperliquid-perp:BTC/USD", "15m", vec![]).is_some());
        assert_eq!(preview_json(Some("bybit-perp:ETHUSDT".into()), Some("4h".into()), Some("14:2".into())), json!({ "market": "bybit-perp:ETHUSDT", "timeframe": "4h", "inputs": [14.0, 2.0] }));
        for (market, tf) in [("BTCUSDT", "1h"), ("Binance:BTC", "1h"), ("binance-perp:BTC<b>", "1h"), ("binance-perp:BTCUSDT", "1 hour"), ("binance-perp:BTCUSDT", "")] {
            assert!(p(market, tf, vec![]).is_none(), "{market} {tf}");
        }
        assert!(p("binance-perp:BTCUSDT", "1h", vec![f64::NAN]).is_none() && p("binance-perp:BTCUSDT", "1h", vec![1.0; tradoor_frost_script::MAX_INPUTS + 1]).is_none());
    }

    #[test]
    fn ids_are_lowercase_letters_and_digits() {
        let id = new_id();
        assert!(valid_id(&id) && id.len() == 12);
        assert_ne!(new_id(), id);
        assert!(!valid_id("../etc") && !valid_id("ABCDEFGH") && !valid_id("abc"));
    }
}
