use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde_json::json;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tradoor_store::DataDir;

use crate::{account, alerts, catalog::Catalog, drawings, flow, footprint, indices, journal, limits, liquidations, open_interest, relay, session, signals, telegram, training, workspaces};

#[derive(Clone)]
pub struct AppState {
    pub pool: deadpool_postgres::Pool,
    pub data: DataDir,
    pub catalog: Arc<Catalog>,
    /// Absent when no ticker's liquidations are being kept.
    pub liquidations: Option<Arc<liquidations::Collector>>,
    /// Absent when no coin's open interest is being recorded.
    pub open_interest: Option<Arc<open_interest::Recorder>>,
    /// The net taker flow of the markets followed.
    pub flow: Option<Arc<flow::Flow>>,
    /// Absent when no Telegram bot token was given.
    pub telegram: Option<Arc<telegram::Telegram>>,
    /// The exchanges' market data, for browsers on networks that block them.
    pub relay: Arc<relay::Relay>,
    /// What one visitor may ask of the server; see `limits`.
    pub limits: Arc<limits::Limits>,
    /// The series this server works out itself.
    pub indices: Arc<indices::Indices>,
    /// Training's live prices, and its engine's nudge.
    pub training: Arc<training::Live>,
    /// Signed-in browsers' sessions, and which ids are accounts'.
    pub sessions: Arc<session::Sessions>,
}

/// An error with the status it answers with. The message goes to the client,
/// so it names the field or the rule — never a connection string.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self(StatusCode::FORBIDDEN, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(e: tokio_postgres::Error) -> Self {
        tracing::error!(error = %e, "database");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        tracing::error!(error = %e, "pool");
        Self(StatusCode::SERVICE_UNAVAILABLE, "database unavailable".into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// The header a browser names itself in: a random id it made and keeps.
pub const OWNER_HEADER: &str = "x-tradoor-owner";

/// Whose drawings and workspace a request is about. Long and random enough to
/// be unguessable: knowing it is what makes them yours.
pub fn owner(headers: &axum::http::HeaderMap) -> ApiResult<String> {
    let id = headers.get(OWNER_HEADER).and_then(|v| v.to_str().ok()).unwrap_or_default();
    if valid_owner(id) {
        Ok(id.to_string())
    } else {
        Err(ApiError::bad_request(format!("{OWNER_HEADER} must be this browser's id: 16 to 64 letters, digits and dashes")))
    }
}

pub fn valid_owner(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Enough of a browser id to tell two apart in a log, and not enough to be one:
/// the whole of it is the only key to that browser's drawings, alerts and
/// Telegram chats, and a log is read by more eyes than a database is.
pub fn short_owner(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

pub fn router(state: AppState, data_dir: PathBuf, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/markets", get(markets))
        .route("/catalog", get(catalog_wire))
        .route("/history", get(history))
        .route("/drawings", get(drawings::list).post(drawings::create).delete(drawings::clear))
        .route("/drawings/{id}", delete(drawings::delete_one).patch(drawings::update))
        .route("/workspaces", get(workspaces::list))
        .route("/workspaces/{id}", get(workspaces::get).put(workspaces::put).delete(workspaces::delete))
        .route("/alerts", get(alerts::list).post(alerts::create).delete(alerts::clear))
        .route("/alerts/restart", axum::routing::post(alerts::restart_all))
        .route("/alerts/log", delete(alerts::clear_log))
        .route("/alerts/{id}", delete(alerts::delete_one).patch(alerts::update))
        .route("/flow", get(flow::state))
        .route("/flow/minutes", get(flow::minutes))
        .route("/signals", get(signals::list))
        .route("/signals/notify", axum::routing::put(signals::set_notify))
        .route("/signals/stats", get(signals::stats))
        .route("/flow/coins/{coin}", axum::routing::put(flow::follow_coin).delete(flow::unfollow_coin))
        .route("/flow/{market}", axum::routing::put(flow::watch))
        .route("/footprint", get(footprint::bars))
        .route("/footprint/markets", get(footprint::recorded))
        .route("/account", get(account::status))
        .route("/account/signin", axum::routing::post(account::sign_in))
        .route("/account/signin/{code}", get(account::collected))
        .route("/account/session", delete(account::sign_out))
        .route("/account/sessions", delete(account::sign_out_everywhere))
        .route("/telegram", get(telegram::status).patch(telegram::set_clock))
        .route("/telegram/link", axum::routing::post(telegram::link))
        .route("/telegram/test", axum::routing::post(telegram::test))
        .route("/telegram/{chat_id}", delete(telegram::unlink))
        .route("/liquidations/{ticker}", get(liquidations::stats))
        .route("/liquidations/{ticker}/live", get(liquidations::live))
        .route("/open-interest", get(open_interest::candles))
        .route("/index/candles", get(indices::candles))
        .route("/index/latest", get(indices::latest))
        .route("/journal", get(journal::list).post(journal::create))
        .route("/journal/reviews", get(journal::reviews))
        .route("/journal/reviews/{week}", axum::routing::put(journal::save_review))
        .route("/journal/{id}", axum::routing::patch(journal::update).delete(journal::delete_one))
        .route("/training", get(training::state))
        .route("/training/orders", axum::routing::post(training::order))
        .route("/training/orders/{id}", axum::routing::delete(training::cancel))
        .route("/training/positions/{symbol}", axum::routing::patch(training::exits))
        .route("/training/positions/{symbol}/close", axum::routing::post(training::close))
        .route("/training/positions/{symbol}/reverse", axum::routing::post(training::reverse))
        .route("/training/leverage/{symbol}", axum::routing::put(training::leverage))
        .route("/training/reset", axum::routing::post(training::reset))
        .route("/training/trades", get(training::history))
        .route("/relay/rest", get(relay::rest))
        .route("/relay/ws", get(relay::ws))
        .layer(middleware::from_fn_with_state(state.clone(), session::middleware))
        .layer(middleware::from_fn_with_state(state.clone(), limits::middleware))
        .layer(CompressionLayer::new())
        .with_state(state);

    let data = Router::new().fallback_service(ServeDir::new(data_dir)).layer(middleware::from_fn(cache_partitions));

    let mut app = Router::new().nest("/api", api).nest("/data", data);
    if let Some(web) = web_dir {
        // The build's own `.br` and `.gz` beside each file, sent as they are.
        let index = ServeFile::new(web.join("index.html")).precompressed_br().precompressed_gzip();
        app = app.fallback_service(ServeDir::new(&web).precompressed_br().precompressed_gzip().fallback(index));
    }
    app.layer(middleware::from_fn(security_headers)).layer(TraceLayer::new_for_http())
}

/// Headers every answer carries, whoever serves the page: nginx sets the same
/// (deploy/nginx/tradoor.conf), this covers the server serving it alone.
async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("strict-origin-when-cross-origin"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    res
}

/// Partition files never change once written, so they are cached for a year;
/// a manifest changes whenever the recorder runs and is always revalidated.
async fn cache_partitions(req: Request, next: Next) -> Response {
    let immutable = req.uri().path().ends_with(".tdc.gz");
    let mut res = next.run(req).await;
    let value = if immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    res
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let db = match state.pool.get().await {
        Ok(client) => client.query_one("SELECT 1", &[]).await.is_ok(),
        Err(_) => false,
    };
    let status = if db { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(json!({ "ok": db, "db": db })))
}

async fn markets(State(state): State<AppState>) -> Response {
    let body = state.catalog.get().await;
    ([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "public, max-age=300")], body.as_ref().clone()).into_response()
}

/// The compact catalogue (see `catalog`): compressed once per refresh, sent so
/// to a browser that takes brotli — every current one — and plain otherwise,
/// for the compression layer to gzip.
async fn catalog_wire(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let wire = state.catalog.wire().await;
    let takes_brotli = headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(',').any(|e| e.trim().starts_with("br")));
    let kept = [(header::CONTENT_TYPE, "application/json"), (header::VARY, "accept-encoding"), (header::CACHE_CONTROL, "public, max-age=300")];
    if takes_brotli && !wire.brotli.is_empty() {
        (kept, [(header::CONTENT_ENCODING, "br")], wire.brotli).into_response()
    } else {
        (kept, wire.json).into_response()
    }
}

/// What history is on disk, per market and base — read from the manifests the
/// browser also reads, so the two can never disagree.
async fn history(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let data = state.data.clone();
    let markets = tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, String> {
        let mut out = Vec::new();
        for market in data.markets().map_err(|e| e.to_string())? {
            let Ok(bytes) = std::fs::read(data.manifest_path(&market)) else { continue };
            let Ok(manifest) = serde_json::from_slice::<tradoor_codec::manifest::Manifest>(&bytes) else { continue };
            let bases: serde_json::Map<String, serde_json::Value> = manifest
                .bases
                .iter()
                .map(|(base, list)| {
                    let rows: u64 = list.iter().map(|p| u64::from(p.rows)).sum();
                    let bytes: u64 = list.iter().map(|p| p.bytes).sum();
                    (base.clone(), json!({ "from": list.first().map(|p| p.first_open), "to": list.last().map(|p| p.last_open), "rows": rows, "bytes": bytes, "files": list.len() }))
                })
                .collect();
            out.push(json!({ "market": manifest.market, "updatedAt": manifest.updated_at, "bases": bases }));
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "markets": markets })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_carries_the_head_of_a_browser_id_and_no_more() {
        assert!(valid_owner("e6a9ab8350808f3e8e1d6ae47f423b31") && !valid_owner("short") && !valid_owner("has spaces in it"));
        assert_eq!(short_owner("e6a9ab8350808f3e8e1d6ae47f423b31"), "e6a9ab83", "eight characters name it without handing it over");
        assert_eq!(short_owner("abc"), "abc", "and a malformed one is not cut mid-character");
    }
}
