//! Frost Scripts: the indicators accounts write, kept on the server so they follow
//! their author to every device. Only an account writes one — a browser that has
//! not signed in with Telegram reads the examples and nothing more.
//!
//! A script is kept as its text and checked here by the formula crate itself
//! (crates/formula), the one the browser runs it with: whether it checks is kept
//! beside it. Nothing of it runs on the server. The moderator, holding the admin
//! key, lists every script and can remove one — kept, with why, but run for no
//! one — restore it, or delete it.
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
}

/// The name trimmed, and both held to their limits.
fn checked(body: &Body) -> ApiResult<(String, bool)> {
    let name: String = body.name.trim().chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(format!("a name is 1 to {MAX_NAME} characters")));
    }
    if body.source.len() > tradoor_formula::MAX_SOURCE {
        return Err(ApiError::bad_request(format!("a script is {} characters at most", tradoor_formula::MAX_SOURCE)));
    }
    Ok((name, tradoor_formula::describe(&body.source).is_ok()))
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
    })
}

const COLUMNS: &str = "id, name, source, checks, updated_at, removed_at, removed_why";

/// `GET /api/scripts`: this account's scripts, the last changed first — a removed one too, saying why.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let rows = state.pool.get().await?.query(&format!("SELECT {COLUMNS} FROM frost_script WHERE owner = $1 ORDER BY updated_at DESC"), &[&owner]).await?;
    Ok(Json(json!({ "scripts": rows.iter().map(row).collect::<Vec<_>>() })))
}

/// `POST /api/scripts`: a new one.
pub async fn create(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Body>) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let (name, checks) = checked(&body)?;
    let client = state.pool.get().await?;
    let held: i64 = client.query_one("SELECT count(*) FROM frost_script WHERE owner = $1", &[&owner]).await?.get(0);
    if held >= MAX_PER_OWNER {
        return Err(ApiError::bad_request(format!("{MAX_PER_OWNER} scripts at most: delete one first")));
    }
    let (id, now) = (new_id(), now_ms());
    let r = client
        .query_one(
            &format!("INSERT INTO frost_script (id, owner, name, source, checks, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6) RETURNING {COLUMNS}"),
            &[&id, &owner, &name, &body.source, &checks, &now],
        )
        .await?;
    Ok(Json(row(&r)))
}

/// `PUT /api/scripts/{id}`: one of this account's, changed. A removed one stays as the moderator left it.
pub async fn update(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Body>) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let (name, checks) = checked(&body)?;
    let client = state.pool.get().await?;
    let r = client
        .query_opt(
            &format!("UPDATE frost_script SET name = $3, source = $4, checks = $5, updated_at = $6 WHERE id = $1 AND owner = $2 AND removed_at IS NULL RETURNING {COLUMNS}"),
            &[&id, &owner, &name, &body.source, &checks, &now_ms()],
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
                "SELECT count(*), count(DISTINCT owner), count(*) FILTER (WHERE removed_at IS NOT NULL), count(*) FILTER (WHERE created_at >= $1) FROM frost_script",
                &[&(now - now.rem_euclid(DAY_MS))],
            )
            .await?;
        anyhow::Ok(json!({ "total": r.get::<_, i64>(0), "authors": r.get::<_, i64>(1), "removed": r.get::<_, i64>(2), "newToday": r.get::<_, i64>(3) }))
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
                    "SELECT s.id, s.name, s.checks, s.created_at, s.updated_at, s.removed_at, s.removed_why, length(s.source), a.name, s.owner
                     FROM frost_script s LEFT JOIN LATERAL (SELECT name FROM account WHERE owner = s.owner ORDER BY since LIMIT 1) a ON true
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

    #[test]
    fn a_name_and_a_text_are_held_to_their_limits() {
        let ok = |name: &str, source: &str| checked(&Body { name: name.into(), source: source.into() });
        assert_eq!(ok("  My RSI ", "plot(close, \"C\")").unwrap(), ("My RSI".to_string(), true));
        assert_eq!(ok("Draft", "x = ema(close)").unwrap().1, false, "kept, and said not to check");
        assert!(ok("", "plot(close)").is_err());
        assert!(ok(&"n".repeat(41), "plot(close)").is_err());
        assert!(ok("Big", &"x".repeat(tradoor_formula::MAX_SOURCE + 1)).is_err());
        assert_eq!(ok("a\u{7}b", "").unwrap().0, "ab", "no control characters in a name");
    }

    #[test]
    fn ids_are_lowercase_letters_and_digits() {
        let id = new_id();
        assert!(valid_id(&id) && id.len() == 12);
        assert_ne!(new_id(), id);
        assert!(!valid_id("../etc") && !valid_id("ABCDEFGH") && !valid_id("abc"));
    }
}
