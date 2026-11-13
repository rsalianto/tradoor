//! What the site's owner tells every reader: one announcement at a time, set on
//! the status page with the admin key and shown as a banner atop the app. Held
//! in memory and answered from there, so a page asking costs no database work;
//! kept in `announcement` for the next start and as a record.
//!
//! It is text and an optional link, nothing else: the page writes it as text,
//! and a link is an https address alone.
use std::sync::{Arc, RwLock};

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tradoor_store::now_ms;

use crate::api::{ApiError, AppState};
use crate::presence::refused;

const MAX_TITLE: usize = 80;
const MAX_BODY: usize = 400;
const MAX_LINK: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Announcement {
    /// When it was put up: a new one has a new id, so a reader who closed the last sees it.
    pub id: i64,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    /// `info` or `warn`.
    pub tone: String,
}

pub struct Board {
    pool: deadpool_postgres::Pool,
    current: RwLock<Option<Announcement>>,
}

impl Board {
    /// The one up when the server last stopped, if any.
    pub async fn load(pool: deadpool_postgres::Pool) -> anyhow::Result<Arc<Self>> {
        let row = pool.get().await?.query_opt("SELECT id, title, body, link, tone FROM announcement WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1", &[]).await?;
        let current = row.map(|r| Announcement { id: r.get(0), title: r.get(1), body: r.get(2), link: r.get(3), tone: r.get(4) });
        Ok(Arc::new(Self { pool, current: RwLock::new(current) }))
    }

    fn now(&self) -> Option<Announcement> {
        self.current.read().ok().and_then(|c| c.clone())
    }
}

#[derive(Deserialize)]
pub struct Draft {
    title: String,
    body: String,
    link: Option<String>,
    tone: Option<String>,
}

fn plain(s: &str) -> String {
    s.trim().chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The draft held to its limits, or why not.
fn checked(d: &Draft) -> Result<(String, String, Option<String>, String), String> {
    let (title, body) = (plain(&d.title), plain(&d.body));
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return Err(format!("a title is 1 to {MAX_TITLE} characters"));
    }
    if body.chars().count() > MAX_BODY {
        return Err(format!("the text is {MAX_BODY} characters at most"));
    }
    let link = d.link.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
    if let Some(l) = &link
        && (!l.starts_with("https://") || l.len() > MAX_LINK || l.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '<' || c == '>'))
    {
        return Err(format!("a link is an https:// address of {MAX_LINK} characters at most"));
    }
    let tone = match d.tone.as_deref() {
        None | Some("info") => "info",
        Some("warn") => "warn",
        Some(_) => return Err("the tone is info or warn".into()),
    };
    Ok((title, body, link, tone.to_string()))
}

/// `GET /api/announcement`: the one up, for every reader; a minute in caches.
pub async fn current(State(state): State<AppState>) -> Response {
    ([(header::CACHE_CONTROL, "public, max-age=60")], Json(json!({ "announcement": state.announcements.now() }))).into_response()
}

/// `GET /api/admin/announcement`: the one up, and the last few, for the status page.
pub async fn read(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let past = async {
        let rows = state.pool.get().await?.query("SELECT id, title, body, link, tone, ended_at FROM announcement ORDER BY id DESC LIMIT 10", &[]).await?;
        anyhow::Ok(rows.iter().map(|r| json!({ "id": r.get::<_, i64>(0), "title": r.get::<_, String>(1), "body": r.get::<_, String>(2), "link": r.get::<_, Option<String>>(3), "tone": r.get::<_, String>(4), "ended": r.get::<_, Option<i64>>(5) })).collect::<Vec<_>>())
    };
    let past = past.await.unwrap_or_default();
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({ "current": state.announcements.now(), "past": past }))).into_response()
}

/// `PUT /api/admin/announcement`: a new one up, in place of the last.
pub async fn publish(State(state): State<AppState>, headers: HeaderMap, Json(draft): Json<Draft>) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let (title, body, link, tone) = match checked(&draft) {
        Ok(v) => v,
        Err(why) => return ApiError::bad_request(why).into_response(),
    };
    let id = now_ms();
    let put = async {
        let client = state.pool.get().await?;
        client.execute("UPDATE announcement SET ended_at = $1 WHERE ended_at IS NULL", &[&id]).await?;
        client.execute("INSERT INTO announcement (id, title, body, link, tone, created_at) VALUES ($1, $2, $3, $4, $5, $1)", &[&id, &title, &body, &link, &tone]).await?;
        anyhow::Ok(())
    };
    if let Err(e) = put.await {
        return ApiError::from(e).into_response();
    }
    let a = Announcement { id, title, body, link, tone };
    if let Ok(mut c) = state.announcements.current.write() {
        *c = Some(a.clone());
    }
    tracing::info!(id, "announcement up");
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({ "current": a }))).into_response()
}

/// `DELETE /api/admin/announcement`: none up.
pub async fn take_down(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let ended = async { anyhow::Ok(state.announcements.pool.get().await?.execute("UPDATE announcement SET ended_at = $1 WHERE ended_at IS NULL", &[&now_ms()]).await?) };
    if let Err(e) = ended.await {
        return ApiError::from(e).into_response();
    }
    if let Ok(mut c) = state.announcements.current.write() {
        *c = None;
    }
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(title: &str, body: &str, link: Option<&str>, tone: Option<&str>) -> Draft {
        Draft { title: title.into(), body: body.into(), link: link.map(str::to_string), tone: tone.map(str::to_string) }
    }

    #[test]
    fn a_draft_is_text_and_an_https_link_alone() {
        let ok = checked(&draft(" tradoor moved ", "Bookmarks\nstill work.", Some("https://tradoorterminal.com/doc.html"), None)).unwrap();
        assert_eq!(ok, ("tradoor moved".into(), "Bookmarks still work.".into(), Some("https://tradoorterminal.com/doc.html".into()), "info".into()));
        assert!(checked(&draft("", "x", None, None)).is_err());
        assert!(checked(&draft(&"t".repeat(81), "", None, None)).is_err());
        assert!(checked(&draft("t", &"b".repeat(401), None, None)).is_err());
        assert!(checked(&draft("t", "", Some("javascript:alert(1)"), None)).is_err(), "no script for an address");
        assert!(checked(&draft("t", "", Some("http://plain.example"), None)).is_err());
        assert!(checked(&draft("t", "", Some("https://a.example/\"><x"), None)).is_err());
        assert!(checked(&draft("t", "", None, Some("loud"))).is_err());
        assert_eq!(checked(&draft("t", "", Some("  "), Some("warn"))).unwrap().2, None, "an empty link is none");
    }
}
