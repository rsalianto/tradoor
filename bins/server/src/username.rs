//! An account's username: what it goes by where others see it — the Frost Scripts
//! it shares — in place of its Telegram name, which it need not give away. The
//! account chooses it once, in Settings, and it never changes: it is bound to
//! the Telegram account for good, so a name seen on a shared script always means
//! the same author. Plain letters, digits and `_`, so that no two look alike.
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tradoor_store::now_ms;

use crate::api::{owner, ApiError, ApiResult, AppState};

/// A username's length, in characters.
const LENGTH: std::ops::RangeInclusive<usize> = 3..=20;
/// Names no account may take, wherever they fall in it and whatever the capitals: tradoor's own, and its moderator's.
const NOT_YOURS: [&str; 7] = ["tradoor", "admin", "moderator", "official", "support", "staff", "telegram"];

/// A username as it is kept, or why it cannot be one.
pub fn checked(raw: &str) -> Result<String, String> {
    let name = raw.trim().trim_start_matches('@');
    if !LENGTH.contains(&name.chars().count()) {
        return Err(format!("a username is {} to {} characters", LENGTH.start(), LENGTH.end()));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("a username is letters a to z, digits and _ only".into());
    }
    if !name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic()) {
        return Err("a username starts with a letter".into());
    }
    let bare = name.replace('_', "").to_ascii_lowercase();
    if NOT_YOURS.iter().any(|w| bare.contains(w)) {
        return Err("that name is tradoor's own: choose another".into());
    }
    Ok(name.to_string())
}

/// The username this account chose, if it has.
pub async fn of(client: &deadpool_postgres::Client, owner: &str) -> ApiResult<Option<String>> {
    Ok(client.query_opt("SELECT name FROM account_username WHERE owner = $1", &[&owner]).await?.map(|r| r.get(0)))
}

#[derive(Deserialize)]
pub struct Claim {
    name: String,
}

/// `PUT /api/account/username`: this account's username, for good. Asked again, it
/// stays what it was; one another account has is refused.
pub async fn claim(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Claim>) -> ApiResult<Json<Value>> {
    let owner = owner(&headers)?;
    if !state.sessions.is_account(&owner) {
        return Err(ApiError::forbidden("sign in with Telegram to choose a username"));
    }
    let name = checked(&body.name).map_err(ApiError::bad_request)?;
    let client = state.pool.get().await?;
    if let Some(had) = of(&client, &owner).await? {
        return Err(ApiError::bad_request(format!("your username is @{had}, for good: it cannot be changed")));
    }
    let Some(user) = client.query_opt("SELECT telegram_user FROM account WHERE owner = $1 ORDER BY since LIMIT 1", &[&owner]).await?.map(|r| r.get::<_, i64>(0)) else {
        return Err(ApiError::forbidden("sign in with Telegram to choose a username"));
    };
    let kept = client
        .query_opt(
            "INSERT INTO account_username (owner, telegram_user, name, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING RETURNING name",
            &[&owner, &user, &name, &now_ms()],
        )
        .await?;
    match kept {
        Some(r) => {
            tracing::info!(owner = crate::api::short_owner(&owner), "username chosen");
            Ok(Json(json!({ "username": r.get::<_, String>(0) })))
        }
        // Two tabs at once, or the name another's.
        None => match of(&client, &owner).await? {
            Some(had) => Ok(Json(json!({ "username": had }))),
            None => Err(ApiError::bad_request(format!("@{name} is someone else's: choose another"))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_is_plain_short_and_not_tradoors() {
        assert_eq!(checked("  @Frost_Fox ").unwrap(), "Frost_Fox");
        assert_eq!(checked("rsa42").unwrap(), "rsa42");
        assert!(checked("ab").is_err() && checked(&"x".repeat(21)).is_err());
        assert!(checked("Ström").is_err(), "letters a to z only: none that look like another's");
        assert!(checked("frost fox").is_err() && checked("a.b").is_err() && checked("<b>x</b>").is_err());
        assert!(checked("_lead").is_err() && checked("4you").is_err());
        for taken in ["tradoor", "Tradoor_Team", "t_r_a_d_o_o_r", "TheAdmin", "Moderator1", "official", "tg_support"] {
            assert!(checked(taken).is_err(), "{taken}");
        }
    }
}
