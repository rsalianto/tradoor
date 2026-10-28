//! The terminal layout, stored whole, one set per browser (the owner the
//! request names). Writes carry the version they were based on, so two tabs
//! editing one workspace get a 409 instead of silently overwriting each other.
//!
//! `default` is the one the app opens and saves as it goes. Any other id is a
//! layout a signed-in reader saved by name, to open again later.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{owner, ApiError, ApiResult, AppState};

const MAX_DOC_BYTES: usize = 512 * 1024;
/// Workspaces one reader keeps: the one in use, and twenty saved.
const MAX_PER_OWNER: i64 = 21;
/// The workspace the app opens; every other id is a saved layout.
const LIVE: &str = "default";

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    if !valid_id(&id) {
        return Err(ApiError::bad_request("workspace ids are lowercase letters, digits and dashes"));
    }
    let client = state.pool.get().await?;
    let row = client.query_opt("SELECT name, doc, version FROM workspace WHERE owner = $1 AND id = $2", &[&owner, &id]).await?;
    let row = row.ok_or_else(|| ApiError::not_found(format!("workspace '{id}' not found")))?;
    Ok(Json(json!({ "id": id, "name": row.get::<_, String>(0), "doc": row.get::<_, Value>(1), "version": row.get::<_, i32>(2) })))
}

#[derive(Deserialize)]
pub struct PutBody {
    name: String,
    doc: Value,
    /// The version this write is based on; 0 creates.
    version: i32,
}

pub async fn put(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<PutBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    let owner = owner(&headers)?;
    if !valid_id(&id) {
        return Err(ApiError::bad_request("workspace ids are lowercase letters, digits and dashes"));
    }
    if !body.doc.is_object() {
        return Err(ApiError::bad_request("doc must be a JSON object"));
    }
    if serde_json::to_vec(&body.doc).map_or(0, |v| v.len()) > MAX_DOC_BYTES {
        return Err(ApiError::bad_request("workspace document is too large"));
    }
    let name = body.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(ApiError::bad_request("name must be 1–120 characters"));
    }
    if id != LIVE && !state.sessions.is_account(&owner) {
        return Err(ApiError::forbidden("sign in with Telegram to save layouts"));
    }
    let client = state.pool.get().await?;
    let updated = client
        .query_opt("UPDATE workspace SET name = $2, doc = $3, version = version + 1, updated_at = now() WHERE id = $1 AND version = $4 AND owner = $5 RETURNING version", &[&id, &name, &body.doc, &body.version, &owner])
        .await?;
    if let Some(row) = updated {
        return Ok((StatusCode::OK, Json(json!({ "version": row.get::<_, i32>(0) }))));
    }
    if body.version == 0 {
        let held: i64 = client.query_one("SELECT count(*) FROM workspace WHERE owner = $1", &[&owner]).await?.get(0);
        if held >= MAX_PER_OWNER {
            return Err(ApiError::bad_request(format!("at most {} saved layouts — delete one first", MAX_PER_OWNER - 1)));
        }
        if held == 0 && !crate::limits::new_owner(&state, &headers) {
            return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many new browser ids from this address; wait a while".into()));
        }
        let created = client.query_opt("INSERT INTO workspace (owner, id, name, doc) VALUES ($4, $1, $2, $3) ON CONFLICT (owner, id) DO NOTHING RETURNING version", &[&id, &name, &body.doc, &owner]).await?;
        if let Some(row) = created {
            return Ok((StatusCode::CREATED, Json(json!({ "version": row.get::<_, i32>(0) }))));
        }
    }
    let current = client.query_opt("SELECT doc, version FROM workspace WHERE owner = $1 AND id = $2", &[&owner, &id]).await?;
    match current {
        Some(row) => Ok((StatusCode::CONFLICT, Json(json!({ "error": "the workspace changed since it was loaded", "doc": row.get::<_, Value>(0), "version": row.get::<_, i32>(1) })))),
        None => Err(ApiError::not_found(format!("workspace '{id}' not found; create it with version 0"))),
    }
}

/// `GET /api/workspaces`: the layouts this reader saved, newest first, without their documents.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let rows = state
        .pool
        .get()
        .await?
        .query(
            "SELECT id, name, version, (extract(epoch FROM updated_at) * 1000)::bigint FROM workspace WHERE owner = $1 AND id <> $2 ORDER BY updated_at DESC",
            &[&owner, &LIVE],
        )
        .await?;
    let layouts: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "id": r.get::<_, String>(0), "name": r.get::<_, String>(1), "version": r.get::<_, i32>(2), "updatedAt": r.get::<_, i64>(3) }))
        .collect();
    Ok(Json(json!({ "layouts": layouts, "max": MAX_PER_OWNER - 1 })))
}

/// `DELETE /api/workspaces/{id}`: a saved layout gone. The one in use is not deleted.
pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    if !valid_id(&id) || id == LIVE {
        return Err(ApiError::bad_request("only a saved layout can be deleted"));
    }
    let n = state.pool.get().await?.execute("DELETE FROM workspace WHERE owner = $1 AND id = $2", &[&owner, &id]).await?;
    if n == 0 {
        return Err(ApiError::not_found(format!("layout '{id}' not found")));
    }
    Ok(Json(json!({ "id": id })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn owners_are_long_random_ids() {
        assert!(crate::api::valid_owner("3f2a9c1e-7b4d-4e8a-9f1c-2d6b8a0e5c7f"));
        assert!(!crate::api::valid_owner("short"));
        assert!(!crate::api::valid_owner("has spaces in it here!"));
        assert!(!crate::api::valid_owner(&"a".repeat(65)));
    }

    #[test]
    fn ids() {
        assert!(super::valid_id("default"));
        assert!(super::valid_id("scalping-2"));
        assert!(!super::valid_id("../x"));
        assert!(!super::valid_id("Default"));
        assert!(!super::valid_id(""));
    }
}
