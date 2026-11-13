//! Accounts: a Telegram user, and the browser id everything of theirs is kept
//! under. Every row — drawings, layouts, watchlists, alerts — stays keyed by
//! that id; an account is how a browser that lost it (history cleared, another
//! device) gets back to it: Sign in with Telegram in Settings, say yes in the
//! bot's chat, and the browser holds a session of the account (see `session`)
//! — never the account's id itself, which alone opens nothing.
//!
//! The browser mints a one-use code and opens the bot with it. The bot asks the
//! Telegram user who opened the link to confirm — a link someone else sent must
//! not sign their browser in as you — and on yes the code holds the account's
//! id until the browser that minted it collects it, once. A user's first sign-in
//! makes the asking browser's id their account's.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};
use tradoor_store::now_ms;

use crate::api::{owner, ApiError, ApiResult, AppState};
use crate::telegram;

/// How long a sign-in link is good for.
const CODE_GOOD_FOR: i64 = 10 * 60_000;
/// What a sign-in link's start parameter begins with, before its code; a link
/// for alerts alone carries a bare code.
pub const START_PREFIX: &str = "in_";

/// A code this browser can spend once to sign in, and a sweep of the stale ones.
async fn mint(pool: &deadpool_postgres::Pool, asker: &str, offset: i32, zone: Option<&str>) -> ApiResult<String> {
    let code = telegram::fresh_code();
    let client = pool.get().await?;
    client.execute("DELETE FROM account_signin WHERE created_at < $1", &[&(now_ms() - CODE_GOOD_FOR)]).await?;
    client
        .execute("INSERT INTO account_signin (code, asker, created_at, utc_offset, zone) VALUES ($1, $2, $3, $4, $5)", &[&code, &asker, &now_ms(), &offset, &zone])
        .await?;
    Ok(code)
}

/// The Telegram user who opened the link: the one asked to confirm, and the
/// only one who can. False if the code is spent, stale, or opened by another.
pub async fn opened(pool: &deadpool_postgres::Pool, code: &str, user: i64) -> anyhow::Result<bool> {
    let client = pool.get().await?;
    let n = client
        .execute(
            "UPDATE account_signin SET telegram_user = $2 WHERE code = $1 AND owner IS NULL AND created_at > $3 AND (telegram_user IS NULL OR telegram_user = $2)",
            &[&code, &user, &(now_ms() - CODE_GOOD_FOR)],
        )
        .await?;
    Ok(n > 0)
}

/// They said yes: their account — made with the asking browser's id if this
/// is their first sign-in — its alerts sent to this chat, and its id held for
/// the browser to collect. None if the code is no longer good.
pub async fn confirm(pool: &deadpool_postgres::Pool, code: &str, user: i64, chat_id: i64, name: Option<&str>) -> anyhow::Result<Option<String>> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    let row = tx
        .query_opt(
            "SELECT asker, utc_offset, zone FROM account_signin WHERE code = $1 AND telegram_user = $2 AND owner IS NULL AND created_at > $3 FOR UPDATE",
            &[&code, &user, &(now_ms() - CODE_GOOD_FOR)],
        )
        .await?;
    let Some(row) = row else { return Ok(None) };
    let (asker, offset, zone): (String, i32, Option<String>) = (row.get(0), row.get(1), row.get(2));
    let owner: String = tx
        .query_one(
            "INSERT INTO account (telegram_user, owner, name, since) VALUES ($1, $2, $3, $4)
             ON CONFLICT (telegram_user) DO UPDATE SET name = COALESCE(EXCLUDED.name, account.name)
             RETURNING owner",
            &[&user, &asker, &name, &now_ms()],
        )
        .await?
        .get(0);
    tx.execute(
        "INSERT INTO telegram_chat (owner, chat_id, name, linked_at, utc_offset, zone) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (owner, chat_id) DO NOTHING",
        &[&owner, &chat_id, &name, &now_ms(), &offset, &zone],
    )
    .await?;
    tx.execute("UPDATE account_signin SET owner = $2 WHERE code = $1", &[&code, &owner]).await?;
    tx.commit().await?;
    tracing::info!(owner = crate::api::short_owner(&owner), "signed in with telegram");
    Ok(Some(owner))
}

/// They said no: the code is spent and nothing else changes.
pub async fn decline(pool: &deadpool_postgres::Pool, code: &str, user: i64) -> anyhow::Result<bool> {
    let client = pool.get().await?;
    Ok(client.execute("DELETE FROM account_signin WHERE code = $1 AND telegram_user = $2 AND owner IS NULL", &[&code, &user]).await? > 0)
}

/// Where a sign-in stands, for the browser that asked.
#[derive(Debug, PartialEq)]
enum Collected {
    /// Not yet confirmed in the chat.
    Waiting,
    /// Confirmed, once: the account's id, its name, and the Telegram user who said yes.
    Done(String, Option<String>, i64),
    /// Declined, stale, or never this browser's.
    Gone,
}

async fn collect(pool: &deadpool_postgres::Pool, code: &str, asker: &str) -> ApiResult<Collected> {
    let client = pool.get().await?;
    let row = client
        .query_opt("SELECT owner, telegram_user FROM account_signin WHERE code = $1 AND asker = $2 AND created_at > $3", &[&code, &asker, &(now_ms() - CODE_GOOD_FOR)])
        .await?;
    let Some(row) = row else { return Ok(Collected::Gone) };
    let (Some(owner), Some(user)) = (row.get::<_, Option<String>>(0), row.get::<_, Option<i64>>(1)) else { return Ok(Collected::Waiting) };
    client.execute("DELETE FROM account_signin WHERE code = $1", &[&code]).await?;
    let name = client.query_opt("SELECT name FROM account WHERE owner = $1", &[&owner]).await?.and_then(|r| r.get(0));
    Ok(Collected::Done(owner, name, user))
}

// ---------------------------------------------------------------- the API

/// `GET /api/account`: whether signing in is on offer here, and the account
/// this browser's id belongs to, if any.
pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let client = state.pool.get().await?;
    let account = client.query_opt("SELECT name, since FROM account WHERE owner = $1 ORDER BY since LIMIT 1", &[&owner]).await?;
    let username = if account.is_some() { crate::username::of(&client, &owner).await? } else { None };
    Ok(Json(json!({
        "telegram": state.telegram.is_some(),
        "bot": state.telegram.as_ref().and_then(|t| t.bot()),
        "account": account.map(|r| json!({ "name": r.get::<_, Option<String>>(0), "since": r.get::<_, i64>(1), "devices": state.sessions.count(&owner), "username": username })),
    })))
}

/// `POST /api/account/signin`: a sign-in code and the bot link that spends
/// it. The body carries the reader's clock, for the chat the account's alerts
/// go to.
pub async fn sign_in(State(state): State<AppState>, headers: HeaderMap, body: Option<Json<Value>>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    let telegram = state.telegram.as_ref().ok_or_else(|| ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE, "this server has no Telegram bot set up".into()))?;
    let bot = telegram.bot().ok_or_else(|| ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE, "the bot has not answered yet — try again in a moment".into()))?;
    let body = body.map_or(Value::Null, |Json(b)| b);
    let (offset, zone) = telegram::clock(&body);
    let code = mint(&state.pool, &owner, offset, zone).await?;
    Ok(Json(json!({ "code": code, "url": format!("https://t.me/{bot}?start={START_PREFIX}{code}"), "goodForMs": CODE_GOOD_FOR })))
}

/// `GET /api/account/signin/{code}`: `waiting` until the reader says yes in
/// the chat, then `done` with a session of the account for this browser —
/// once — or `gone`.
pub async fn collected(State(state): State<AppState>, headers: HeaderMap, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    Ok(Json(match collect(&state.pool, &code, &owner).await? {
        Collected::Waiting => json!({ "state": "waiting" }),
        Collected::Done(account, name, user) => {
            let agent = headers.get(axum::http::header::USER_AGENT).and_then(|v| v.to_str().ok());
            let session = state.sessions.open(&account, user, agent).await.map_err(|e| {
                tracing::error!(error = %e, "opening a session");
                ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE, "signed in, but the session could not be kept — sign in again".into())
            })?;
            tracing::info!(owner = crate::api::short_owner(&account), "session opened");
            json!({ "state": "done", "session": session, "name": name })
        }
        Collected::Gone => json!({ "state": "gone" }),
    }))
}

/// `DELETE /api/account/session`: this browser signs out; the account and
/// every other browser signed in to it stay as they are.
pub async fn sign_out(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<axum::http::StatusCode> {
    let token = headers.get(crate::session::SESSION_HEADER).and_then(|v| v.to_str().ok()).ok_or_else(|| ApiError::bad_request("this browser is not signed in"))?;
    state.sessions.close(token).await.map_err(|e| {
        tracing::error!(error = %e, "closing a session");
        ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE, "could not sign out — try again".into())
    })?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// `DELETE /api/account/sessions`: every browser signed in to the account,
/// this one too, is signed out — for a phone lost or a session seen.
pub async fn sign_out_everywhere(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    if headers.get(crate::session::SESSION_HEADER).is_none() {
        return Err(ApiError::bad_request("this browser is not signed in"));
    }
    let n = state.sessions.close_all(&owner).await.map_err(|e| {
        tracing::error!(error = %e, "closing sessions");
        ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE, "could not sign out — try again".into())
    })?;
    tracing::info!(owner = crate::api::short_owner(&owner), sessions = n, "signed out everywhere");
    Ok(Json(json!({ "signedOut": n })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a Postgres at DATABASE_URL with the schema applied; run with
    /// `cargo test -p tradoor-server -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a Postgres at DATABASE_URL"]
    async fn a_sign_in_hands_the_account_to_the_browser_that_asked_once_confirmed() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = deadpool_postgres::Config { url: Some(url), ..Default::default() }.create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls).unwrap();
        const FIRST: &str = "zz-test-account-first";
        const LATER: &str = "zz-test-account-later";
        let (user, stranger) = (4_242_424_201_i64, 4_242_424_202_i64);
        let client = pool.get().await.unwrap();
        client.execute("DELETE FROM account WHERE telegram_user = ANY($1)", &[&vec![user, stranger]]).await.unwrap();
        client.execute("DELETE FROM telegram_chat WHERE chat_id = ANY($1)", &[&vec![user, stranger]]).await.unwrap();

        // The first sign-in makes the asking browser's id the account's.
        let code = mint(&pool, FIRST, 8 * 60, Some("Asia/Singapore")).await.unwrap();
        assert_eq!(collect(&pool, &code, FIRST).await.unwrap(), Collected::Waiting);
        assert!(opened(&pool, &code, user).await.unwrap());
        assert!(!opened(&pool, &code, stranger).await.unwrap(), "only the one who opened it is asked");
        assert_eq!(confirm(&pool, &code, stranger, stranger, None).await.unwrap(), None, "nor can another confirm it");
        assert_eq!(confirm(&pool, &code, user, user, Some("ricky")).await.unwrap().as_deref(), Some(FIRST));
        assert_eq!(collect(&pool, &code, LATER).await.unwrap(), Collected::Gone, "another browser cannot collect it");
        assert_eq!(collect(&pool, &code, FIRST).await.unwrap(), Collected::Done(FIRST.into(), Some("ricky".into()), user));
        assert_eq!(collect(&pool, &code, FIRST).await.unwrap(), Collected::Gone, "and it is handed over once");
        let linked: i64 = client.query_one("SELECT count(*) FROM telegram_chat WHERE owner = $1 AND chat_id = $2", &[&FIRST, &user]).await.unwrap().get(0);
        assert_eq!(linked, 1, "the account's alerts reach the chat");

        // A later browser, history cleared, takes the account's id back.
        let code = mint(&pool, LATER, 0, None).await.unwrap();
        assert!(opened(&pool, &code, user).await.unwrap());
        assert_eq!(confirm(&pool, &code, user, user, Some("ricky")).await.unwrap().as_deref(), Some(FIRST));
        assert_eq!(collect(&pool, &code, LATER).await.unwrap(), Collected::Done(FIRST.into(), Some("ricky".into()), user));

        // A no spends the code and changes nothing.
        let code = mint(&pool, LATER, 0, None).await.unwrap();
        assert!(opened(&pool, &code, user).await.unwrap());
        assert!(decline(&pool, &code, user).await.unwrap());
        assert_eq!(collect(&pool, &code, LATER).await.unwrap(), Collected::Gone);

        client.execute("DELETE FROM account WHERE telegram_user = ANY($1)", &[&vec![user, stranger]]).await.unwrap();
        client.execute("DELETE FROM telegram_chat WHERE chat_id = ANY($1)", &[&vec![user, stranger]]).await.unwrap();
    }
}
