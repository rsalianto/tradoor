//! The indicators an account starred in the picker. Only an account has them
//! here: a browser that has not signed in keeps its stars to itself, and brings
//! them along when it does.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};
use tradoor_store::now_ms;

use crate::api::{owner, ApiError, ApiResult, AppState};

/// Stars one account keeps: the catalogue is far shorter, and rows are not free.
const MAX_PER_OWNER: i64 = 64;

/// A study's kind or a layer's id. The server does not read the catalogue, so a
/// star from a newer page is kept: an older one simply lists nothing for it.
fn valid_kind(kind: &str) -> bool {
    !kind.is_empty() && kind.len() <= 40 && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn account(state: &AppState, headers: &HeaderMap) -> ApiResult<String> {
    let owner = owner(headers)?;
    if state.sessions.is_account(&owner) {
        Ok(owner)
    } else {
        Err(ApiError::forbidden("sign in with Telegram to keep favourites on every device"))
    }
}

/// `GET /api/account/favourites`: what this account starred, oldest star first.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = account(&state, &headers)?;
    let rows = state.pool.get().await?.query("SELECT kind FROM indicator_favourite WHERE owner = $1 ORDER BY added_at, kind", &[&owner]).await?;
    Ok(Json(json!({ "kinds": rows.iter().map(|r| r.get::<_, String>(0)).collect::<Vec<_>>() })))
}

/// `PUT /api/account/favourites/{kind}`: starred; again changes nothing.
pub async fn add(State(state): State<AppState>, headers: HeaderMap, Path(kind): Path<String>) -> ApiResult<StatusCode> {
    let owner = account(&state, &headers)?;
    if !valid_kind(&kind) {
        return Err(ApiError::bad_request("an indicator is letters, digits, dashes and underscores"));
    }
    let client = state.pool.get().await?;
    let held: i64 = client.query_one("SELECT count(*) FROM indicator_favourite WHERE owner = $1", &[&owner]).await?.get(0);
    if held >= MAX_PER_OWNER {
        return Err(ApiError::bad_request(format!("at most {MAX_PER_OWNER} favourites — take one off first")));
    }
    client.execute("INSERT INTO indicator_favourite (owner, kind, added_at) VALUES ($1, $2, $3) ON CONFLICT (owner, kind) DO NOTHING", &[&owner, &kind, &now_ms()]).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/account/favourites/{kind}`: not starred any more; already so is fine.
pub async fn remove(State(state): State<AppState>, headers: HeaderMap, Path(kind): Path<String>) -> ApiResult<StatusCode> {
    let owner = account(&state, &headers)?;
    state.pool.get().await?.execute("DELETE FROM indicator_favourite WHERE owner = $1 AND kind = $2", &[&owner, &kind]).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #[test]
    fn kinds() {
        assert!(super::valid_kind("stochrsi"));
        assert!(super::valid_kind("script-3f2a_1"));
        assert!(!super::valid_kind(""));
        assert!(!super::valid_kind("ema:50"), "a kind, not one study's parameters");
        assert!(!super::valid_kind("../x"));
        assert!(!super::valid_kind(&"a".repeat(41)));
    }
}
