//! tradoor messenger: one room every reader sees and anyone may write in.
//!
//! The newest messages are held in memory and answered from there, so reading
//! costs no database work; each is kept in `messenger_message` too, for the next
//! start, and the oldest let go past `KEEP`. A page asks for what is newer than
//! the last it has, every few seconds while its messenger is open.
//!
//! Anyone may write, signed in or not, so what keeps the room fit to read is
//! here, on the server:
//! - **No links.** A message that reads as an address in any spelling — a scheme,
//!   `www`, a name and a domain ending, written with odd dots, letters that only
//!   look Latin, characters that cannot be seen, or `dot` spelled out — is refused.
//!   What is sent is put on the page as text, never as markup, so nothing in it
//!   can be clicked whatever it says.
//! - **Plain text.** Control characters, characters that turn text around and
//!   ones that cannot be seen are taken out (emoji keep their joiners), marks
//!   stacked on a letter are kept to two, and a message is a few lines at most.
//! - **Limits.** A few messages a minute from a browser and from an address, the
//!   same text not twice in a row, and a cap on the whole room.
//! - **A name that is the sender's.** A signed-in account with a username writes
//!   as `@username`; anyone else as a guest name worked out from the browser's id.
//! - **A moderator.** On the status page, with the admin key: a message taken
//!   down, its sender's taken down too, and the sender muted for so many hours,
//!   by browser id and, while the server runs, by address. No address is ever
//!   written to disk.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tradoor_store::now_ms;

use crate::api::{owner, short_owner, ApiError, ApiResult, AppState};
use crate::limits::{self, Rate};
use crate::presence::refused;

/// Messages kept, in memory and on disk.
pub const KEEP: usize = 200;
/// A message's characters, and its lines.
pub const MAX_CHARS: usize = 300;
const MAX_LINES: usize = 4;
/// Marks stacked on one letter.
const MAX_MARKS: usize = 2;
/// From one browser, from one address, and from everyone together.
const SENDER: Rate = Rate { name: "messenger", per_min: 12.0, burst: 4.0 };
const ADDRESS: Rate = Rate { name: "messenger-address", per_min: 24.0, burst: 8.0 };
const EVERYONE: Rate = Rate { name: "messenger-all", per_min: 300.0, burst: 60.0 };
/// The same text again from one sender within this long is refused.
const SAME_TEXT_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const MAX_MUTE_HOURS: i64 = 24 * 365;
const MAX_WHY: usize = 200;
/// Addresses and senders held in memory at most, so no one can fill it.
const MAX_HELD: usize = 20_000;

#[derive(Clone, Debug)]
struct Message {
    id: i64,
    owner: String,
    name: String,
    signed: bool,
    text: String,
    at: i64,
}

impl Message {
    /// What a reader sees: never the sender's browser id.
    fn public(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "signed": self.signed, "text": self.text, "at": self.at })
    }
}

#[derive(Default)]
struct Room {
    recent: VecDeque<Message>,
    /// Messages taken down lately, so a page that has them lets them go.
    gone: VecDeque<i64>,
    /// The address each recent message came from: in memory alone, for a mute.
    addresses: HashMap<i64, String>,
    /// Addresses muted, until when: in memory alone, gone on a restart.
    muted: HashMap<String, i64>,
    /// Each sender's last text and when, so the same is not sent twice running.
    last: HashMap<String, (String, i64)>,
}

pub struct Messenger {
    pool: deadpool_postgres::Pool,
    room: Mutex<Room>,
}

impl Messenger {
    /// The newest messages as the server last kept them.
    pub async fn load(pool: deadpool_postgres::Pool) -> anyhow::Result<Arc<Self>> {
        let rows = pool.get().await?.query("SELECT id, owner, name, signed, text, at FROM messenger_message ORDER BY id DESC LIMIT $1", &[&(KEEP as i64)]).await?;
        let recent = rows.iter().rev().map(|r| Message { id: r.get(0), owner: r.get(1), name: r.get(2), signed: r.get(3), text: r.get(4), at: r.get(5) }).collect();
        Ok(Arc::new(Self { pool, room: Mutex::new(Room { recent, ..Room::default() }) }))
    }

    fn room(&self) -> MutexGuard<'_, Room> {
        self.room.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes messages down, from memory and disk, and says so to the pages that have them.
    async fn take_down(&self, ids: &[i64]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.pool.get().await?.execute("DELETE FROM messenger_message WHERE id = ANY($1)", &[&ids]).await?;
        let mut room = self.room();
        room.recent.retain(|m| !ids.contains(&m.id));
        for id in ids {
            room.gone.push_back(*id);
        }
        while room.gone.len() > KEEP {
            room.gone.pop_front();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- text

/// Characters taken out of a message: controls but the line break, ones that
/// turn text around, and ones that cannot be seen. Emoji keep the joiner
/// (U+200D) and the variation selectors that make them what they are.
fn unwanted(c: char) -> bool {
    (c.is_control() && c != '\n')
        || matches!(c, '\u{200B}' | '\u{200C}' | '\u{200E}' | '\u{200F}' | '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}')
        || ('\u{202A}'..='\u{202E}').contains(&c)
        || ('\u{2060}'..='\u{2064}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
        || ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// Marks drawn over the letter before them: what a pile of them is made of.
fn mark(c: char) -> bool {
    ('\u{0300}'..='\u{036F}').contains(&c) || ('\u{1AB0}'..='\u{1AFF}').contains(&c) || ('\u{1DC0}'..='\u{1DFF}').contains(&c) || ('\u{20D0}'..='\u{20FF}').contains(&c) || ('\u{FE20}'..='\u{FE2F}').contains(&c)
}

/// A message as it is kept, or why it is not one.
pub fn cleaned(raw: &str) -> Result<String, &'static str> {
    let mut text = String::with_capacity(raw.len());
    let mut marks = 0;
    for c in raw.replace("\r\n", "\n").chars().filter(|c| !unwanted(*c)) {
        if mark(c) {
            marks += 1;
            if marks > MAX_MARKS {
                continue;
            }
        } else {
            marks = 0;
        }
        text.push(c);
    }
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut kept: Vec<&str> = Vec::new();
    for line in lines {
        if line.trim().is_empty() && kept.last().is_none_or(|l: &&str| l.trim().is_empty()) {
            continue;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    if kept.len() > MAX_LINES {
        return Err("a message is 4 lines at most");
    }
    let text = kept.join("\n").trim().to_string();
    if text.is_empty() {
        return Err("write something first");
    }
    if text.chars().count() > MAX_CHARS {
        return Err("a message is 300 characters at most");
    }
    if has_link(&text) {
        return Err("links can't be sent in tradoor messenger");
    }
    Ok(text)
}

/// Endings read as an address even with spaces round the dot, or `dot` spelled
/// out: plainly a site's, and none a word a trader writes after "dot" (DOT is a
/// coin: "dot to the moon" is talk, not a link).
const ENDINGS: &[&str] = &["com", "net", "org", "io", "gg", "xyz", "ly", "app", "me", "co", "ru", "cn", "info", "biz", "site", "online", "link", "top", "club", "vip", "shop", "store", "finance", "exchange"];

/// The text as a link would be read in it: lower case, with full-width letters,
/// odd dots and letters that only look Latin made plain, and invisible joiners gone.
fn skeleton(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{200D}' | '\u{FE0E}' | '\u{FE0F}') && !unwanted(*c) && !mark(*c))
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '。' | '｡' | '︒' | '﹒' | '․' | '‧' | '·' | '•' | '∙' | '⋅' | '٫' | '۔' => '.',
            'а' | 'ɑ' | 'α' => 'a',
            'е' | 'ε' | 'ё' => 'e',
            'о' | 'ο' | 'օ' => 'o',
            'р' | 'ρ' => 'p',
            'с' | 'ϲ' => 'c',
            'у' | 'γ' => 'y',
            'х' | 'χ' => 'x',
            'і' | 'ı' | 'ι' => 'i',
            'ј' => 'j',
            'ѕ' => 's',
            'ԁ' => 'd',
            'ɡ' => 'g',
            'ո' => 'n',
            'м' => 'm',
            'т' => 't',
            'к' | 'κ' => 'k',
            'ν' => 'v',
            'һ' => 'h',
            'ⅼ' => 'l',
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace("(.)", ".")
        .replace("[.]", ".")
        .replace("{.}", ".")
        .replace("<.>", ".")
}

/// Whether the text holds an address in any spelling of one.
pub fn has_link(text: &str) -> bool {
    let s = skeleton(text);
    if ["http", "hxxp", "www.", "www ", "://", "ftp.", ".onion", "t.me", "wa.me"].iter().any(|w| s.contains(w)) {
        return true;
    }
    let chars: Vec<char> = s.chars().collect();
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let letters_from = |i: usize| -> String { chars[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect() };
    let mut digits_dots = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        // A run of dotted numbers: an address by number.
        if c.is_ascii_digit() || c == '.' {
            digits_dots += 1;
            let run: String = chars[i + 1 - digits_dots..=i].iter().collect();
            if run.split('.').filter(|p| !p.is_empty() && p.chars().all(|d| d.is_ascii_digit())).count() >= 4 && run.matches('.').count() >= 3 {
                return true;
            }
        } else {
            digits_dots = 0;
        }
        if c != '.' {
            continue;
        }
        // A name, a dot and an ending: `name.com`, `name . com`.
        let mut left = i;
        while left > 0 && chars[left - 1] == ' ' && i - left < 2 {
            left -= 1;
        }
        let spaced_left = left != i;
        let named = left > 0 && word(chars[left - 1]);
        let mut right = i + 1;
        while right < chars.len() && chars[right] == ' ' && right - i <= 2 {
            right += 1;
        }
        let spaced_right = right != i + 1;
        if !named || right >= chars.len() {
            continue;
        }
        let ending = letters_from(right);
        let ends_there = chars.get(right + ending.len()).is_none_or(|c| !c.is_ascii_alphabetic());
        if ending.len() >= 2 && ends_there && (!(spaced_left || spaced_right) || ENDINGS.contains(&ending.as_str())) {
            return true;
        }
    }
    // `name dot com`, `name (dot) com`.
    for dot in [" dot ", " d0t ", "(dot)", "[dot]", "{dot}"] {
        let mut from = 0;
        while let Some(at) = s[from..].find(dot) {
            let start = from + at;
            let after = s[start + dot.len()..].trim_start();
            let ending: String = after.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            let before = s[..start].trim_end();
            if before.chars().last().is_some_and(word) && ENDINGS.contains(&ending.as_str()) {
                return true;
            }
            from = start + dot.len();
        }
    }
    false
}

/// A guest's name, the same for a browser every time and nothing of its id.
fn guest(owner: &str) -> String {
    let hash = Sha256::digest(format!("tradoor-messenger:{owner}").as_bytes());
    format!("guest-{:02x}{:02x}", hash[0], hash[1])
}

// ---------------------------------------------------------------- the API

#[derive(Deserialize)]
pub struct After {
    after: Option<i64>,
}

/// `GET /api/messenger?after=<id>`: the messages newer than a page has, those
/// taken down lately, and how many have tradoor on screen.
pub async fn read(State(state): State<AppState>, Query(q): Query<After>) -> Response {
    let after = q.after.unwrap_or(0);
    let (messages, gone) = {
        let room = state.messenger.room();
        (room.recent.iter().filter(|m| m.id > after).map(Message::public).collect::<Vec<_>>(), room.gone.iter().copied().collect::<Vec<_>>())
    };
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({ "messages": messages, "gone": gone, "online": state.presence.online().max(1) }))).into_response()
}

#[derive(Deserialize)]
pub struct Draft {
    text: String,
}

/// `POST /api/messenger`: a message from this browser.
pub async fn send(State(state): State<AppState>, headers: HeaderMap, Json(draft): Json<Draft>) -> ApiResult<Response> {
    let owner = owner(&headers)?;
    let address = limits::client(&headers).unwrap_or_default();
    let text = cleaned(&draft.text).map_err(ApiError::bad_request)?;
    let now = now_ms();
    {
        let room = state.messenger.room();
        if room.muted.get(&address).is_some_and(|until| *until > now) {
            return Err(ApiError::forbidden("you can't send messages for now"));
        }
        if room.last.get(&owner).is_some_and(|(last, at)| *last == text && now - at < SAME_TEXT_MS) {
            return Err(ApiError::bad_request("that was just sent"));
        }
    }
    if !state.limits.take(SENDER, &owner) || (!address.is_empty() && !state.limits.take(ADDRESS, &address)) || !state.limits.take(EVERYONE, "all") {
        return Ok(limits::too_many("messages"));
    }
    let client = state.pool.get().await?;
    if client.query_opt("SELECT 1 FROM messenger_mute WHERE owner = $1 AND until > $2", &[&owner, &now]).await?.is_some() {
        return Err(ApiError::forbidden("you can't send messages for now"));
    }
    let username = if state.sessions.is_account(&owner) { crate::username::of(&client, &owner).await? } else { None };
    let (name, signed) = match username {
        Some(n) => (format!("@{n}"), true),
        None => (guest(&owner), false),
    };
    let id: i64 = client
        .query_one("INSERT INTO messenger_message (owner, name, signed, text, at) VALUES ($1, $2, $3, $4, $5) RETURNING id", &[&owner, &name, &signed, &text, &now])
        .await?
        .get(0);
    // The oldest let go: ids only grow, so all below this are older than the newest KEEP.
    if let Err(e) = client.execute("DELETE FROM messenger_message WHERE id <= $1", &[&(id - KEEP as i64)]).await {
        tracing::warn!(error = %e, "messenger: the oldest messages were not let go");
    }
    let message = Message { id, owner: owner.clone(), name, signed, text: text.clone(), at: now };
    let public = message.public();
    {
        let mut room = state.messenger.room();
        room.recent.push_back(message);
        while room.recent.len() > KEEP {
            if let Some(old) = room.recent.pop_front() {
                room.addresses.remove(&old.id);
            }
        }
        if !address.is_empty() && room.addresses.len() < MAX_HELD {
            room.addresses.insert(id, address);
        }
        if room.last.len() >= MAX_HELD {
            room.last.retain(|_, (_, at)| now - *at < SAME_TEXT_MS);
        }
        if room.last.len() < MAX_HELD {
            room.last.insert(owner, (text, now));
        }
    }
    Ok((StatusCode::CREATED, Json(public)).into_response())
}

/// `GET /api/admin/messenger`: the recent messages for the moderator, the newest
/// first, each with the start of its sender's id to tell senders apart.
pub async fn moderate_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let room = state.messenger.room();
    let list: Vec<Value> = room
        .recent
        .iter()
        .rev()
        .map(|m| json!({ "id": m.id, "name": m.name, "signed": m.signed, "text": m.text, "at": m.at, "sender": short_owner(&m.owner) }))
        .collect();
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({ "messages": list }))).into_response()
}

#[derive(Deserialize)]
pub struct Action {
    /// Take down every recent message of this one's sender, not this one alone.
    #[serde(default)]
    all: bool,
    /// Mute the sender for so many hours; none, or 0, mutes no one.
    hours: Option<i64>,
    why: Option<String>,
}

/// `POST /api/admin/messenger/{id}`: a message taken down, or all its sender's,
/// and its sender muted for so many hours.
pub async fn moderate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>, Json(a): Json<Action>) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let found = {
        let room = state.messenger.room();
        room.recent.iter().find(|m| m.id == id).map(|m| (m.owner.clone(), room.addresses.get(&id).cloned()))
    };
    let Some((sender, address)) = found else {
        return ApiError::not_found("no such message: taken down already, or older than the newest 200").into_response();
    };
    let ids: Vec<i64> = if a.all { state.messenger.room().recent.iter().filter(|m| m.owner == sender).map(|m| m.id).collect() } else { vec![id] };
    let done = async {
        state.messenger.take_down(&ids).await?;
        let hours = a.hours.unwrap_or(0).clamp(0, MAX_MUTE_HOURS);
        if hours > 0 {
            let until = now_ms() + hours * HOUR_MS;
            let why: Option<String> = a.why.map(|w| w.trim().chars().filter(|c| !c.is_control()).take(MAX_WHY).collect::<String>()).filter(|w| !w.is_empty());
            state
                .pool
                .get()
                .await?
                .execute("INSERT INTO messenger_mute (owner, until, why) VALUES ($1, $2, $3) ON CONFLICT (owner) DO UPDATE SET until = EXCLUDED.until, why = EXCLUDED.why", &[&sender, &until, &why])
                .await?;
            if let Some(address) = address {
                let mut room = state.messenger.room();
                if room.muted.len() >= MAX_HELD {
                    let now = now_ms();
                    room.muted.retain(|_, u| *u > now);
                }
                room.muted.insert(address, until);
            }
        }
        anyhow::Ok(ids.len())
    };
    match done.await {
        Ok(n) => Json(json!({ "takenDown": n })).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "messenger: moderating");
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "the database did not answer".into()).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_refused_in_any_spelling() {
        for link in [
            "see https://example.com",
            "go to www.scam.xyz",
            "example.com",
            "join t.me/pumpgroup",
            "discord.gg/abc123",
            "bit.ly/x",
            "my site: scam . com",
            "scam dot com",
            "scam (dot) io",
            "scam[.]net",
            "ｓｃａｍ．ｃｏｍ",
            "scam。com",
            "sсam.com",
            "sca\u{200B}m.com",
            "go\u{200D}ogle.com",
            "192.168.0.1",
            "hxxp://bad",
            "Visit MySite.Finance",
            "wa.me/123",
            "scam  .  com",
        ] {
            assert!(has_link(link), "{link}");
            assert!(cleaned(link).is_err(), "{link}");
        }
    }

    #[test]
    fn plain_talk_and_tradoors_own_names_are_not_links() {
        for talk in [
            "BTC to 70k? 🚀🚀",
            "BTC.D and USDT.D both up, OTHERS.D down",
            "e.g. a long at 67,000.50 with a stop at 66.5k",
            "i.e. wait for the close.",
            "Nice. Thanks for that!",
            "RSI 14 at 26.11, Stoch 11.51 / 12.94",
            "version 0.8.3 is out",
            "@trader_one what do you think?",
            "👍🏽 👨‍👩‍👧 ❤️",
            "1.5x leverage, 2.25 R",
            "buy dot to the moon, dot in profit",
            "polkadot today and polkadot news",
            "Nice . To the moon",
        ] {
            assert!(!has_link(talk), "{talk}");
            assert!(cleaned(talk).is_ok(), "{talk}");
        }
    }

    #[test]
    fn a_message_is_plain_text_of_a_few_lines() {
        assert_eq!(cleaned("  hello \r\n\n\n\nthere  ").unwrap(), "hello\n\nthere");
        assert_eq!(cleaned("a\u{202E}b\u{200B}c\u{0007}").unwrap(), "abc", "no turning text round, nothing unseen, no controls");
        assert_eq!(cleaned("👨\u{200D}👩\u{200D}👧 ❤\u{FE0F}").unwrap(), "👨\u{200D}👩\u{200D}👧 ❤\u{FE0F}", "emoji keep their joiners");
        assert_eq!(cleaned("e\u{301}\u{301}\u{301}\u{301}\u{301}").unwrap().chars().count(), 3, "two marks on a letter at most");
        assert!(cleaned("   \n  ").is_err());
        assert!(cleaned("1\n2\n3\n4\n5").is_err());
        assert!(cleaned(&"x".repeat(MAX_CHARS + 1)).is_err());
        assert!(cleaned(&"x".repeat(MAX_CHARS)).is_ok());
    }

    #[test]
    fn a_guest_name_is_the_same_for_a_browser_and_says_nothing_of_its_id() {
        let id = "abcdefghij0123456789";
        assert_eq!(guest(id), guest(id));
        assert_ne!(guest(id), guest("zyxwvutsrq9876543210"));
        assert!(guest(id).starts_with("guest-") && guest(id).len() == 10);
        assert!(!guest(id).contains("abcd"));
    }
}
