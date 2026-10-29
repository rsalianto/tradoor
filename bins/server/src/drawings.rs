//! Hand-drawn markup, stored per browser and market. The timeframe it was drawn
//! on is kept, but every timeframe of the market shows it: anchors are epoch ms
//! and price, which mean the same thing on any bar length — as on TradingView.
//! Every query is scoped to the owner the request names, so one browser can
//! neither see nor change another's — and to the chart, candles or TPO, which
//! keep separate drawings.
//!
//! Strict about the SHAPE of what goes in: `points` is JSONB, so Postgres would
//! accept any JSON at all, and a row that cannot be rendered cannot be selected
//! and deleted from the chart either — it becomes junk only psql can remove.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tradoor_core::ChartTimeframe;
use tradoor_venues::MarketRef;

use crate::api::{owner, ApiError, ApiResult, AppState};

pub const KINDS: [&str; 13] = ["path", "trendline", "horizontal", "ray", "rect", "fib", "text", "measure", "long", "short", "brush", "channel", "circle"];
/// A style is a flat object of short settings; this bounds what one row may carry.
pub const MAX_STYLE_BYTES: usize = 8 * 1024;
pub const MAX_POINTS: usize = 2000;
/// Drawings one browser keeps, over every market: far past any hand's work.
pub const MAX_PER_OWNER: i64 = 20_000;

/// Anchors each kind needs, and whether that is a minimum (path, brush) rather
/// than an exact count.
pub fn arity(kind: &str) -> Option<(usize, bool)> {
    Some(match kind {
        "horizontal" | "ray" | "text" | "circle" => (1, false),
        "trendline" | "rect" | "fib" | "measure" => (2, false),
        "long" | "short" | "channel" => (3, false),
        "path" | "brush" => (2, true),
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    pub time: f64,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingInput {
    pub kind: String,
    pub points: Vec<Point>,
    pub style: Map<String, Value>,
    pub label: Option<String>,
}

/// Validate a drawing off the wire. The error names the field and the rule; it
/// becomes the 400's message.
pub fn parse_body(body: &Value) -> Result<DrawingInput, String> {
    let b = body.as_object().ok_or("body must be a JSON object")?;
    let kind = b.get("kind").and_then(Value::as_str).filter(|k| KINDS.contains(k)).ok_or_else(|| {
        format!("invalid kind: '{}'. Valid: {}", b.get("kind").map_or("undefined".into(), |k| k.as_str().map_or(k.to_string(), str::to_string)), KINDS.join(", "))
    })?;
    let points = parse_points(kind, b.get("points"))?;
    let style = parse_style(b.get("style"))?.unwrap_or_default();
    let label = b.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
    if kind == "text" && label.is_none() {
        return Err("a text drawing needs a label".into());
    }
    Ok(DrawingInput { kind: kind.to_string(), points, style, label })
}

/// A style object: flat, scalar values only, and small. `None` when absent; a
/// non-object is read as no style, as before.
pub fn parse_style(value: Option<&Value>) -> Result<Option<Map<String, Value>>, String> {
    let Some(style) = value.and_then(Value::as_object) else { return Ok(None) };
    if let Some((key, _)) = style.iter().find(|(_, v)| v.is_object() || v.is_array()) {
        return Err(format!("style.{key} must be a string, number, boolean or null"));
    }
    if serde_json::to_string(style).map_or(0, |s| s.len()) > MAX_STYLE_BYTES {
        return Err(format!("style may be at most {MAX_STYLE_BYTES} bytes"));
    }
    Ok(Some(style.clone()))
}

/// The anchors of a `kind`, counted and checked.
pub fn parse_points(kind: &str, value: Option<&Value>) -> Result<Vec<Point>, String> {
    let points = value.and_then(Value::as_array).ok_or("points must be an array")?;
    let (expected, open_ended) = arity(kind).ok_or_else(|| format!("unknown kind: {kind}"))?;
    if open_ended {
        if points.len() < expected {
            return Err(format!("a {kind} needs at least {expected} points, got {}", points.len()));
        }
        if points.len() > MAX_POINTS {
            return Err(format!("a {kind} may carry at most {MAX_POINTS} points, got {}", points.len()));
        }
    } else if points.len() != expected {
        return Err(format!("a {kind} needs exactly {expected} point{}, got {}", if expected == 1 { "" } else { "s" }, points.len()));
    }
    let points = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let p = p.as_object().ok_or_else(|| format!("points[{i}] must be an object with time and price"))?;
            let finite = |key: &str| p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
            let time = finite("time").ok_or_else(|| format!("points[{i}].time must be a finite epoch-ms number"))?;
            let price = finite("price").ok_or_else(|| format!("points[{i}].price must be a finite number"))?;
            Ok(Point { time, price })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(points)
}

#[derive(Deserialize)]
pub struct Scope {
    market: String,
    timeframe: String,
    /// `candles` (the default), `tpo`, or `training` — the training chart's own.
    #[serde(default)]
    chart: Option<String>,
}

/// The market a chart shows: one, or one priced in another — `venue:A/venue:B`,
/// two different markets — whose drawings are its own.
pub fn chart_market(raw: &str) -> Option<String> {
    if let Some(market) = MarketRef::parse(raw) {
        return Some(market.to_string());
    }
    let (a, b) = raw.split_once('/')?;
    let (a, b) = (MarketRef::parse(a)?.to_string(), MarketRef::parse(b)?.to_string());
    (a != b).then(|| format!("{a}/{b}"))
}

impl Scope {
    fn validated(&self) -> ApiResult<(String, &str, &str)> {
        let market = chart_market(&self.market).ok_or_else(|| ApiError::bad_request(format!("not a market: '{}' (expected venue:SYMBOL, or venue:A/venue:B)", self.market)))?;
        ChartTimeframe::parse(&self.timeframe).ok_or_else(|| ApiError::bad_request(format!("invalid timeframe: '{}'", self.timeframe)))?;
        let chart = match self.chart.as_deref() {
            None | Some("candles") => "candles",
            Some("tpo") => "tpo",
            Some("training") => "training",
            Some(other) => return Err(ApiError::bad_request(format!("chart is candles, tpo or training, not '{other}'"))),
        };
        Ok((market, &self.timeframe, chart))
    }
}

pub(crate) const COLUMNS: &str = "id, market, timeframe, kind, points, style, label, (extract(epoch FROM created_at) * 1000)::int8";

pub(crate) fn row_json(r: &tokio_postgres::Row) -> Value {
    json!({
        "id": r.get::<_, i64>(0), "market": r.get::<_, String>(1), "timeframe": r.get::<_, String>(2), "kind": r.get::<_, String>(3),
        "points": r.get::<_, Value>(4), "style": r.get::<_, Value>(5), "label": r.get::<_, Option<String>>(6), "createdAt": r.get::<_, i64>(7),
    })
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap, Query(scope): Query<Scope>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let (market, _, chart) = scope.validated()?;
    let client = state.pool.get().await?;
    let rows = client.query(&format!("SELECT {COLUMNS} FROM drawing WHERE owner = $1 AND market = $2 AND chart = $3 ORDER BY id"), &[&owner, &market, &chart]).await?;
    // Kinds this version no longer draws are left out rather than shipped to a
    // renderer that cannot show — or delete — them.
    let drawings: Vec<Value> = rows.iter().filter(|r| KINDS.contains(&r.get::<_, &str>(3))).map(row_json).collect();
    Ok(Json(json!({ "count": drawings.len(), "drawings": drawings })))
}

pub async fn insert(client: &tokio_postgres::Client, owner: &str, market: &str, timeframe: &str, chart: &str, input: &DrawingInput) -> Result<tokio_postgres::Row, tokio_postgres::Error> {
    let points = serde_json::to_value(&input.points).expect("points serialise");
    let style = Value::Object(input.style.clone());
    client
        .query_one(
            &format!("INSERT INTO drawing (owner, market, timeframe, chart, kind, points, style, label) VALUES ($1, $2, $3, $8, $4, $5, $6, $7) RETURNING {COLUMNS}"),
            &[&owner, &market, &timeframe, &input.kind, &points, &style, &input.label, &chart],
        )
        .await
}

pub async fn create(State(state): State<AppState>, headers: HeaderMap, Query(scope): Query<Scope>, Json(body): Json<Value>) -> ApiResult<(StatusCode, Json<Value>)> {
    let owner = owner(&headers)?;
    let (market, timeframe, chart) = scope.validated()?;
    let input = parse_body(&body).map_err(ApiError::bad_request)?;
    let client = state.pool.get().await?;
    let held: i64 = client.query_one("SELECT count(*) FROM drawing WHERE owner = $1", &[&owner]).await?.get(0);
    if held >= MAX_PER_OWNER {
        return Err(ApiError::bad_request(format!("a browser may keep at most {MAX_PER_OWNER} drawings")));
    }
    if held == 0 && !crate::limits::new_owner(&state, &headers) {
        return Err(ApiError(axum::http::StatusCode::TOO_MANY_REQUESTS, "too many new browser ids from this address; wait a while".into()));
    }
    let row = insert(&client, &owner, &market, timeframe, chart, &input).await?;
    let drawing = row_json(&row);
    // A training chart's positions are drawings, not trades in the journal.
    if chart != "training" {
        journal(&client, &owner, &drawing).await;
    }
    Ok((StatusCode::CREATED, Json(json!({ "drawing": drawing }))))
}

/// Move, relabel or restyle one drawing. Its kind and its market never change.
pub async fn update(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>, Json(body): Json<Value>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let b = body.as_object().ok_or_else(|| ApiError::bad_request("body must be a JSON object"))?;
    let client = state.pool.get().await?;
    // Someone else's drawing is not found, not forbidden: its existence is not theirs to learn.
    let row = client.query_opt("SELECT kind, label, chart FROM drawing WHERE id = $1 AND owner = $2", &[&id, &owner]).await?;
    let row = row.ok_or_else(|| ApiError::not_found(format!("drawing {id} not found")))?;
    let kind: String = row.get(0);
    let chart: String = row.get(2);

    let points = match b.get("points") {
        Some(value) => {
            let points = parse_points(&kind, Some(value)).map_err(ApiError::bad_request)?;
            Some(serde_json::to_value(points).expect("points serialise"))
        }
        None => None,
    };
    let label = match b.get("label") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => {
            let text = value.as_str().map(str::trim).filter(|l| !l.is_empty()).ok_or_else(|| ApiError::bad_request("label must be a non-empty string, or null to clear it"))?;
            Some(Some(text.to_string()))
        }
    };
    let style = match b.get("style") {
        None => None,
        Some(value) if value.is_object() => parse_style(Some(value)).map_err(ApiError::bad_request)?.map(Value::Object),
        Some(_) => return Err(ApiError::bad_request("style must be an object")),
    };
    if points.is_none() && label.is_none() && style.is_none() {
        return Err(ApiError::bad_request("nothing to change: send points, label or style"));
    }
    if kind == "text" && matches!(label, Some(None)) {
        return Err(ApiError::bad_request("a text drawing needs a label"));
    }
    let label = label.unwrap_or_else(|| row.get(1));

    let row = client
        .query_one(
            &format!("UPDATE drawing SET points = COALESCE($2::jsonb, points), label = $3, style = COALESCE($4::jsonb, style) WHERE id = $1 AND owner = $5 RETURNING {COLUMNS}"),
            &[&id, &points, &label, &style, &owner],
        )
        .await?;
    let drawing = row_json(&row);
    if chart != "training" {
        journal(&client, &owner, &drawing).await;
    }
    Ok(Json(json!({ "drawing": drawing })))
}

/// A long or short position is a trade in the journal too: kept in step with
/// it here. A journal that cannot be written does not cost the drawing.
async fn journal(client: &tokio_postgres::Client, owner: &str, drawing: &Value) {
    if let Err(e) = crate::journal::sync_drawing(client, owner, drawing).await {
        tracing::warn!(error = %e, "keeping a position's trade in the journal");
    }
}

pub async fn delete_one(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let client = state.pool.get().await?;
    match client.execute("DELETE FROM drawing WHERE id = $1 AND owner = $2", &[&id, &owner]).await? {
        0 => Err(ApiError::not_found(format!("drawing {id} not found"))),
        _ => Ok(Json(json!({ "id": id }))),
    }
}

/// Every drawing on one market's chart — on every timeframe, since each shows them all.
pub async fn clear(State(state): State<AppState>, headers: HeaderMap, Query(scope): Query<Scope>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let (market, _, chart) = scope.validated()?;
    let client = state.pool.get().await?;
    let deleted = client.execute("DELETE FROM drawing WHERE owner = $1 AND market = $2 AND chart = $3", &[&owner, &market, &chart]).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ratio_chart_keeps_drawings_of_its_own() {
        assert_eq!(chart_market("binance-perp:BTCUSDT").as_deref(), Some("binance-perp:BTCUSDT"));
        assert_eq!(chart_market("bybit-perp:XPLUSDT/binance-spot:BTCUSDT").as_deref(), Some("bybit-perp:XPLUSDT/binance-spot:BTCUSDT"));
        assert_eq!(chart_market("binance-perp:BTCUSDT/binance-perp:BTCUSDT"), None, "a market over itself is no ratio");
        assert_eq!(chart_market("XPLUSDT/BTCUSDT"), None, "each side names its venue");
        assert_eq!(chart_market("binance-perp:BTCUSDT/"), None);
    }

    fn point() -> Value {
        json!({ "time": 1_780_000_000_000i64, "price": 62_000 })
    }

    fn rejects(body: Value, needle: &str) {
        let err = parse_body(&body).expect_err("expected a rejection");
        assert!(err.contains(needle), "{err:?} does not contain {needle:?}");
    }

    #[test]
    fn accepts_and_keeps_only_time_and_price() {
        let ok = parse_body(&json!({ "kind": "trendline", "points": [point(), { "time": 1, "price": 63_000 }] })).unwrap();
        assert_eq!((ok.kind.as_str(), ok.points.len()), ("trendline", 2));
        let ok = parse_body(&json!({ "kind": "horizontal", "points": [{ "time": 1, "price": 2, "colour": "red" }] })).unwrap();
        assert_eq!(ok.points, vec![Point { time: 1.0, price: 2.0 }]);
    }

    #[test]
    fn names_what_is_wrong() {
        rejects(json!(null), "must be a JSON object");
        rejects(json!({ "kind": "gann", "points": [point()] }), "invalid kind: 'gann'");
        rejects(json!({ "points": [point()] }), "invalid kind");
        rejects(json!({ "kind": "text", "points": [point()] }), "needs a label");
        rejects(json!({ "kind": "text", "points": [point()], "label": "   " }), "needs a label");
        rejects(json!({ "kind": "path", "points": [point()] }), "needs at least 2 points");
        rejects(json!({ "kind": "brush", "points": vec![point(); 2001] }), "at most 2000 points");
        rejects(json!({ "kind": "long", "points": [point(), point()] }), "needs exactly 3 points");
        rejects(json!({ "kind": "horizontal", "points": [point(), point()] }), "needs exactly 1 point, got 2");
        rejects(json!({ "kind": "horizontal", "points": [42] }), "points[0] must be an object");
        rejects(json!({ "kind": "horizontal", "points": "nope" }), "points must be an array");
        rejects(json!({ "kind": "horizontal", "points": [{ "time": 1 }] }), "price must be a finite");
        rejects(json!({ "kind": "horizontal", "points": [{ "time": "1", "price": 1 }] }), "time must be a finite");
        rejects(json!({ "kind": "trendline", "points": [point(), { "time": 1, "price": "x" }] }), "points[1].price");
    }

    #[test]
    fn style_and_label_normalise() {
        assert!(parse_body(&json!({ "kind": "horizontal", "points": [point()], "style": [1, 2] })).unwrap().style.is_empty());
        assert_eq!(parse_body(&json!({ "kind": "horizontal", "points": [point()], "style": { "color": "#fff", "width": 2 } })).unwrap().style.len(), 2);
        rejects(json!({ "kind": "horizontal", "points": [point()], "style": { "levels": [1] } }), "style.levels must be");
        rejects(json!({ "kind": "channel", "points": [point(), point()] }), "needs exactly 3 points");
        assert_eq!(parse_body(&json!({ "kind": "horizontal", "points": [point()], "label": " R1 " })).unwrap().label.as_deref(), Some("R1"));
        assert_eq!(parse_body(&json!({ "kind": "horizontal", "points": [point()], "label": "  " })).unwrap().label, None);
        for kind in KINDS {
            assert!(arity(kind).is_some_and(|(n, _)| n > 0), "{kind}");
        }
    }
}
