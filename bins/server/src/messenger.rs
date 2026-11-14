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
//! - **Limits.** A request is 4 KB at most, read no further. A few messages a
//!   minute from a browser (fewer for a guest) and from an address, the same text
//!   not twice in a row, and a cap on all guests together, so a flood of them
//!   cannot silence the accounts.
//! - **A brake.** The moderator can let only signed-in accounts with a username
//!   write, kept across restarts, while guests flood.
//! - **No bait.** What phishing is made of when no link can be sent — asking
//!   for a seed phrase or a key, "verify your wallet", an airdrop to claim, a
//!   giveaway, "DM me" or a WhatsApp number, someone posing as support — is refused.
//! - **No wallets.** A wallet address — Ethereum's and every EVM chain's, Bitcoin's,
//!   TRON's, Solana's and the like — or a long hex string (a key, a hash) is
//!   refused: "send to this address" is the scam a link cannot carry.
//! - **Reports.** A reader can report a message; the moderator sees how many did.
//! - **A name that is the sender's.** A signed-in account with a username writes
//!   as `@username`; anyone else as a guest name worked out from the browser's id.
//! - **A moderator.** On the status page, with the admin key: a message taken
//!   down, its sender's taken down too, and the sender muted for so many hours,
//!   by browser id and, while the server runs, by address. No address is ever
//!   written to disk.
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
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
/// A request's body, read no further: a message is far less.
pub const MAX_BODY: usize = 4 * 1024;
/// From an account, from a guest's browser, from one address, and from every guest together.
const ACCOUNT: Rate = Rate { name: "messenger", per_min: 12.0, burst: 4.0 };
const GUEST: Rate = Rate { name: "messenger-guest", per_min: 6.0, burst: 3.0 };
const ADDRESS: Rate = Rate { name: "messenger-address", per_min: 20.0, burst: 6.0 };
const GUESTS: Rate = Rate { name: "messenger-guests", per_min: 240.0, burst: 40.0 };
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
    /// A fingerprint of each sender's last text and when, so the same is not sent twice running.
    last: HashMap<String, (u64, i64)>,
    /// Who reported each recent message, by browser id: each counted once.
    reports: HashMap<i64, HashSet<String>>,
}

/// A text's fingerprint: enough to tell a repeat, and a few bytes however long the text.
fn print(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

pub struct Messenger {
    pool: deadpool_postgres::Pool,
    room: Mutex<Room>,
    /// Only signed-in accounts with a username may write: the moderator's brake.
    accounts_only: AtomicBool,
}

impl Messenger {
    /// The newest messages as the server last kept them.
    pub async fn load(pool: deadpool_postgres::Pool) -> anyhow::Result<Arc<Self>> {
        let rows = pool.get().await?.query("SELECT id, owner, name, signed, text, at FROM messenger_message ORDER BY id DESC LIMIT $1", &[&(KEEP as i64)]).await?;
        let recent = rows.iter().rev().map(|r| Message { id: r.get(0), owner: r.get(1), name: r.get(2), signed: r.get(3), text: r.get(4), at: r.get(5) }).collect();
        let braked = pool.get().await?.query_opt("SELECT value FROM messenger_setting WHERE key = 'accounts_only'", &[]).await?.is_some_and(|r| r.get::<_, String>(0) == "true");
        Ok(Arc::new(Self { pool, room: Mutex::new(Room { recent, ..Room::default() }), accounts_only: AtomicBool::new(braked) }))
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
    // Far past what any message of MAX_CHARS could be, even in four-byte characters with marks: not worked through.
    if raw.len() > MAX_CHARS * 12 {
        return Err("a message is 300 characters at most");
    }
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
    if wallet(&text) {
        return Err("wallet addresses and keys can't be sent in tradoor messenger");
    }
    if bait(&text) {
        return Err("that reads like a scam, so it can't be sent: tradoor never asks anyone for keys, seed phrases or logins");
    }
    Ok(text)
}

/// What a scam asks for, when it cannot send a link: keys and phrases, a wallet
/// to "verify", something free to claim, a talk taken elsewhere, someone posing
/// as support. Read in the text's plain form, spaces made single.
const BAIT: &[&str] = &[
    "seed phrase", "seedphrase", "recovery phrase", "secret phrase", "secret recovery", "mnemonic", "private key", "privatekey", "12 words", "24 words", "12-word", "24-word",
    "verify your wallet", "validate your wallet", "sync your wallet", "connect your wallet", "wallet connect", "walletconnect", "rectify your wallet", "unlock your wallet",
    "claim your airdrop", "claim airdrop", "airdrop claim", "free airdrop", "claim your reward", "claim reward", "giveaway", "give away", "double your", "send 1 get 2", "send and receive",
    "guaranteed profit", "guaranteed return", "100% profit", "risk free profit", "recover your funds", "funds recovery", "recover lost", "investment plan",
    "dm me", "d m me", "pm me", "inbox me", "message me on", "text me on", "whatsapp", "whats app", "telegram me", "add me on", "contact me on",
    "contact support", "contact admin", "support team", "customer support", "customer service", "help desk", "account manager", "official support", "tradoor support", "tradoor team", "tradoor admin",
    "frasa pemulihan", "kunci pribadi", "kunci privat", "hubungi admin", "chat admin", "hubungi saya", "japri", "klaim airdrop", "verifikasi dompet",
];

/// Base58's letters: every digit but 0, every letter but `0 O I l`.
fn base58(c: char) -> bool {
    c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l')
}

/// Whether the text holds a wallet address or a key, whole or broken up by spaces and marks.
pub fn wallet(text: &str) -> bool {
    // The text as words of letters and digits, each with the one mark before it, if one.
    let mut tokens: Vec<(String, Option<char>)> = Vec::new();
    let mut word = String::new();
    let mut before: Option<char> = None;
    let mut marks = 0;
    for c in text.chars().filter(|c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')) {
        if c.is_ascii_alphanumeric() {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            tokens.push((std::mem::take(&mut word), if marks == 1 { before } else { None }));
            marks = 0;
        }
        marks += 1;
        before = Some(c);
    }
    if !word.is_empty() {
        tokens.push((word, if marks == 1 { before } else { None }));
    }
    // A piece of an address: digits in it, or letters in both cases as no word has.
    let piece = |t: &str| t.len() >= 3 && (t.chars().any(|c| c.is_ascii_digit()) || (t.chars().filter(char::is_ascii_uppercase).count() >= 2 && t.chars().any(|c| c.is_ascii_lowercase())));
    // Pieces apart by one space or mark read as one: `0x742d 35Cc…`, `bc1q-xy2k…`.
    let mut runs: Vec<String> = Vec::new();
    for (t, mark) in &tokens {
        let joins = mark.is_some_and(|m| " -.:_*,/|".contains(m)) && piece(t) && runs.last().is_some_and(|r| piece(r));
        match runs.last_mut() {
            Some(r) if joins => r.push_str(t),
            _ => runs.push(t.clone()),
        }
    }
    runs.iter().any(|r| {
        let lower = r.to_ascii_lowercase();
        let hex_after = |at: usize| lower[at..].chars().take_while(char::is_ascii_hexdigit).count();
        // Ethereum and every EVM chain: 0x and forty hex digits.
        if lower.match_indices("0x").any(|(at, _)| hex_after(at + 2) >= 38) {
            return true;
        }
        // A key or a hash: forty hex digits or more, with letters among them as no number has.
        if lower.len() >= 40 && lower.chars().all(|c| c.is_ascii_hexdigit()) && lower.chars().any(|c| c.is_ascii_alphabetic()) && lower.chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
        // Bitcoin's and Litecoin's bech32: bc1… / ltc1…, and a long tail.
        if (lower.starts_with("bc1") || lower.starts_with("ltc1")) && lower.len() >= 25 {
            return true;
        }
        // Base58 — Bitcoin's 1… and 3…, TRON's T…, Solana's, XRP's and more: long, both cases and digits, as no word is.
        r.len() >= 25 && r.chars().all(base58) && r.chars().any(|c| c.is_ascii_uppercase()) && r.chars().any(|c| c.is_ascii_lowercase()) && r.chars().any(|c| c.is_ascii_digit())
    })
}

/// Whether the text asks for what a scam does: a phrase of `BAIT` standing as words of its own.
pub fn bait(text: &str) -> bool {
    let plain = skeleton(text).split_whitespace().collect::<Vec<_>>().join(" ");
    let bounded = |at: usize, len: usize| {
        let before = plain[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = plain[at + len..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    };
    BAIT.iter().any(|b| plain.match_indices(b).any(|(at, _)| bounded(at, b.len())))
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
        // A dot, a slash or a colon written as code: `&#46;`, `%2e`, `\x2e`.
        .replace("&#46;", ".")
        .replace("&#x2e;", ".")
        .replace("&period;", ".")
        .replace("&dot;", ".")
        .replace("%2e", ".")
        .replace("\\x2e", ".")
        .replace("\\u002e", ".")
        .replace("\\u{2e}", ".")
        .replace("&#47;", "/")
        .replace("%2f", "/")
        .replace("&#58;", ":")
        .replace("%3a", ":")
        .replace("(.)", ".")
        .replace("[.]", ".")
        .replace("{.}", ".")
        .replace("<.>", ".")
}

/// Whether the text holds an address in any spelling of one.
pub fn has_link(text: &str) -> bool {
    let s = skeleton(text);
    if ["http", "hxxp", "www.", "www ", "://", "ftp.", ".onion", "t.me", "wa.me", "xn--"].iter().any(|w| s.contains(w)) {
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
        // `name.c0m`: an ending with digits for letters, read against the endings alone.
        if ENDINGS.contains(&leet(&chars[right..]).as_str()) {
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
    // A name and an ending with anything else between: `name[]com`, `name , com`, `name com`, `name-com`.
    if separated(&chars) {
        return true;
    }
    false
}

/// Endings read past a separator of spaces alone, or a dash: none a word anyone writes.
const BARE: &[&str] = &["com", "xyz", "biz"];
/// Endings read past punctuation with spaces round it: plainly a site's, and not
/// a word one writes after a full stop ("lol, gg" and "thanks! info soon" are talk).
const STRONG: &[&str] = &["com", "org", "io", "xyz", "ly", "ru", "cn", "biz"];
/// Endings read when spelt out a letter at a time: `c o m`, `c*o*m`, `c.o.m`.
const SPREAD: &[&str] = &["com", "org", "net", "xyz"];
/// How long a run between a name and its ending is read: past this it is two thoughts, not an address.
const MAX_GAP: usize = 24;

/// What can stand for a dot between a name and its ending: anything but a plain
/// Latin letter or digit, a space, and what words are made with (`don't`,
/// `long-term`, `snake_case`, `long/short`). Symbols, punctuation, emoji and
/// letters of other scripts all count, at any length up to `MAX_GAP`.
fn separator(c: char) -> bool {
    !c.is_ascii_alphanumeric() && !c.is_whitespace() && !"'-_/".contains(c)
}

/// The letters and digits from the start of `chars`, digits that stand for
/// letters read as them: `c0m` is `com`, `n3t` is `net`. Compared with the endings
/// alone, never read as a word, so `1.5x` and `v2.0` stay what they are.
fn leet(chars: &[char]) -> String {
    let run: String = chars.iter().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if !run.chars().any(|c| c.is_ascii_alphabetic()) {
        return String::new();
    }
    run.chars().map(|c| match c {
        '0' => 'o',
        '1' => 'l',
        '3' => 'e',
        '4' => 'a',
        '5' => 's',
        '7' => 't',
        _ => c,
    }).collect()
}

/// An ending spelt a letter at a time from `at`: single letters, each apart by
/// one space or mark, read back together; none where the letters run on.
fn spread(chars: &[char], at: usize) -> String {
    let mut out = String::new();
    let mut i = at;
    while i < chars.len() && chars[i].is_ascii_alphabetic() {
        // A letter followed by another letter is a word, not a spelt-out ending.
        if chars.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric()) {
            return String::new();
        }
        out.push(chars[i]);
        match chars.get(i + 1) {
            Some(c) if (separator(*c) || *c == ' ') && chars.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) => i += 2,
            _ => break,
        }
    }
    out
}

/// Whether a name and an ending stand apart by something other than a dot: what
/// is between them decides which endings count, so talk such as "gross, net",
/// "yes, me too" or "BTC,ETH,SOL" stays talk.
fn separated(chars: &[char]) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric();
    let mut i = 0;
    while i < chars.len() {
        if !word(chars[i]) || (i + 1 < chars.len() && word(chars[i + 1])) {
            i += 1;
            continue;
        }
        // chars[i] ends a word: what follows, up to the next letter, is the gap.
        let name_len = chars[..=i].iter().rev().take_while(|c| word(**c)).count();
        let mut j = i + 1;
        while j < chars.len() && !chars[j].is_ascii_alphanumeric() && j - i <= MAX_GAP {
            j += 1;
        }
        if j >= chars.len() || j == i + 1 || !chars[j].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let gap = &chars[i + 1..j];
        let ending: String = chars[j..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
        let ends_there = chars.get(j + ending.len()).is_none_or(|c| !c.is_ascii_alphabetic());
        // `name.c o m`, `name c*o*m`: the ending spelt out after any gap.
        let spelt = spread(chars, j);
        if spelt.len() >= 3 && SPREAD.contains(&spelt.as_str()) && (name_len >= 3 || gap.iter().any(|c| separator(*c))) {
            return true;
        }
        // `name*c0m`: digits for letters, where punctuation alone stands between.
        if !gap.iter().any(|c| c.is_whitespace()) && gap.iter().all(|c| separator(*c)) && ENDINGS.contains(&leet(&chars[j..]).as_str()) {
            return true;
        }
        if ends_there {
            let spaces = gap.iter().any(|c| c.is_whitespace());
            let marks = gap.iter().filter(|c| separator(**c)).count();
            let only = |f: &dyn Fn(char) -> bool| gap.iter().all(|c| f(*c));
            let hit = if marks > 0 && !spaces && only(&|c| separator(c)) {
                // `name[]com`, `name,net`: punctuation alone between, any ending.
                ENDINGS.contains(&ending.as_str())
            } else if marks > 0 && only(&|c| separator(c) || c.is_whitespace()) {
                // `name [ ] com`, `name , com`: punctuation and spaces, an ending that is plainly a site's.
                STRONG.contains(&ending.as_str())
            } else if only(&|c| c == ' ') && gap.len() <= 3 {
                // `name com`: spaces alone, an ending no one writes as a word, after a name of some length.
                name_len >= 3 && BARE.contains(&ending.as_str())
            } else if only(&|c| c == '-' || c == '_' || c == '/') && gap.len() == 1 {
                // `name-com`, `name/com`.
                name_len >= 4 && BARE.contains(&ending.as_str())
            } else {
                false
            };
            if hit {
                return true;
            }
        }
        i = j;
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
    let accounts_only = state.messenger.accounts_only.load(Ordering::Relaxed);
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({ "messages": messages, "gone": gone, "online": state.presence.online().max(1), "accountsOnly": accounts_only }))).into_response()
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
    let fingerprint = print(&text);
    {
        let room = state.messenger.room();
        if room.muted.get(&address).is_some_and(|until| *until > now) {
            return Err(ApiError::forbidden("you can't send messages for now"));
        }
        if room.last.get(&owner).is_some_and(|(last, at)| *last == fingerprint && now - at < SAME_TEXT_MS) {
            return Err(ApiError::bad_request("that was just sent"));
        }
    }
    let account = state.sessions.is_account(&owner);
    if state.messenger.accounts_only.load(Ordering::Relaxed) && !account {
        return Err(ApiError::forbidden("only signed-in accounts with a username can write for now"));
    }
    // A guest is held to less, and all guests together to a cap the accounts are not under.
    let allowed = if account { state.limits.take(ACCOUNT, &owner) } else { state.limits.take(GUEST, &owner) && state.limits.take(GUESTS, "all") };
    if !allowed || (!address.is_empty() && !state.limits.take(ADDRESS, &address)) {
        return Ok(limits::too_many("messages"));
    }
    let client = state.pool.get().await?;
    if client.query_opt("SELECT 1 FROM messenger_mute WHERE owner = $1 AND until > $2", &[&owner, &now]).await?.is_some() {
        return Err(ApiError::forbidden("you can't send messages for now"));
    }
    let username = if account { crate::username::of(&client, &owner).await? } else { None };
    if username.is_none() && state.messenger.accounts_only.load(Ordering::Relaxed) {
        return Err(ApiError::forbidden("only signed-in accounts with a username can write for now"));
    }
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
            room.last.insert(owner, (fingerprint, now));
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
        .map(|m| json!({ "id": m.id, "name": m.name, "signed": m.signed, "text": m.text, "at": m.at, "sender": short_owner(&m.owner), "reports": room.reports.get(&m.id).map_or(0, HashSet::len) }))
        .collect();
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({ "messages": list, "accountsOnly": state.messenger.accounts_only.load(Ordering::Relaxed) }))).into_response()
}

/// Reports from one browser: a few a minute.
const REPORTER: Rate = Rate { name: "messenger-report", per_min: 10.0, burst: 5.0 };

/// `POST /api/messenger/{id}/report`: this browser reports a message to the moderator, once.
pub async fn report(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let owner = owner(&headers)?;
    if !state.limits.take(REPORTER, &owner) {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many reports: wait a minute".into()));
    }
    let mut room = state.messenger.room();
    if !room.recent.iter().any(|m| m.id == id) {
        return Err(ApiError::not_found("no such message: taken down already"));
    }
    if room.reports.len() >= KEEP * 2 {
        let live: HashSet<i64> = room.recent.iter().map(|m| m.id).collect();
        room.reports.retain(|id, _| live.contains(id));
    }
    let who = room.reports.entry(id).or_default();
    if who.len() < 1_000 {
        who.insert(owner);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct Mode {
    #[serde(rename = "accountsOnly")]
    accounts_only: bool,
}

/// `POST /api/admin/messenger-mode`: the brake on or off — only accounts with a username write — kept across restarts.
pub async fn moderate_mode(State(state): State<AppState>, headers: HeaderMap, Json(m): Json<Mode>) -> Response {
    if let Some(answer) = refused(&state, &headers) {
        return answer;
    }
    let value = if m.accounts_only { "true" } else { "false" };
    let kept = async {
        state.pool.get().await?.execute("INSERT INTO messenger_setting (key, value) VALUES ('accounts_only', $1) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value", &[&value]).await?;
        anyhow::Ok(())
    };
    match kept.await {
        Ok(()) => {
            state.messenger.accounts_only.store(m.accounts_only, Ordering::Relaxed);
            Json(json!({ "accountsOnly": m.accounts_only })).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "messenger: the brake");
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "the database did not answer".into()).into_response()
        }
    }
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
            "xn--80ak6aa92e.com",
            "visit xn--e1afmkfd",
            "tradoorterminal[]com",
            "tradoorterminal[ ]com",
            "tradoorterminal ( ) com",
            "tradoorterminal{}io",
            "tradoorterminal,com",
            "tradoorterminal , com",
            "tradoorterminal;net",
            "tradoorterminal|xyz",
            "tradoorterminal*com",
            "tradoorterminal★com",
            "tradoorterminal com",
            "tradoorterminal-com",
            "tradoorterminal/com",
            "scam<>gg",
            "tradoorterminal🔥com",
            "tradoorterminal*^%$#@#$^&@com",
            "tradoor*^%$#@#$^&@terminal*^%$#@#$^&@com",
            "tradoorterminal@com",
            "tradoorterminal$com",
            "tradoorterminal%&com",
            "tradoorterminal!?com",
            "tradoorterminal.*com",
            "tradoorterminal中com",
            "t^r^a^d^o^o^r^.com",
            "tradoorterminal.c o m",
            "tradoorterminal c*o*m",
            "tradoorterminal . c.o.m",
            "t r a d o o r . c o m",
            "tradoorterminal(d)(o)(t)com",
            "tradoor terminal ! ! com",
            "tradoorterminal.c0m",
            "tradoorterminal[.]c0m",
            "tradoorterminal*n3t",
            "tradoorterminal&#46;com",
            "tradoorterminal%2ecom",
            "tradoorterminal\\x2ecom",
            "tradoorterminal&period;com",
            "https&#58;&#47;&#47;tradoorterminal",
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
            "gross, net profit was fine",
            "yes, me too",
            "BTC,ETH,SOL pumping",
            "long/short ratio, long-term view",
            "nice gg everyone",
            "e-com and dot-com bubble",
            "TP1 (67k), TP2 [68k]",
            "Don't get liquidated folks. enjoy your weekend",
            "BTC🚀ETH🚀SOL",
            "lol, gg",
            "thanks! info soon",
            "50%off fees, Q&A later, R&D, AT&T",
            "it's 10:30am, don't fomo",
            "I like c o m p o u n d interest",
            "welcome to the telecom outcome",
            "BTC/USDT and TP1/TP2",
            "#BTC #ETH trending",
            "price $67k, +5% today!!!",
            "1.5x leverage and v2.0, 2.5k",
            "0.8.3 is out, 3.7 RR",
        ] {
            assert!(!has_link(talk), "{talk}");
            assert!(cleaned(talk).is_ok(), "{talk}");
        }
    }

    #[test]
    fn phishing_bait_is_refused_and_talk_is_not() {
        for scam in [
            "send me your seed phrase to unlock",
            "Verify your wallet to claim",
            "Claim your AIRDROP now",
            "huge giveaway today",
            "DM me for signals",
            "D M  me",
            "contact support on whatsapp",
            "I am from tradoor support",
            "double your BTC",
            "kirim frasa pemulihan kamu",
            "japri aja",
        ] {
            assert!(bait(scam), "{scam}");
            assert!(cleaned(scam).is_err(), "{scam}");
        }
        for talk in ["send me your best setup", "wallet is safe on a hardware device", "the support level at 66k held", "airdrop season soon?", "double top on the 4h", "Ottawa meetup this week", "admin meme lol", "giveaways? no"] {
            assert!(!bait(talk), "{talk}");
        }
    }

    #[test]
    fn wallet_addresses_and_keys_are_refused_and_talk_is_not() {
        for w in [
            "send to 0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
            "CA: 0x742d35cc6634c0532925a3b844bc454e4438f44e",
            "0x742d 35Cc 6634 C053 2925 a3b8 44Bc 454e 4438 f44e",
            "0x742d-35Cc-6634-C053-2925-a3b8-44Bc-454e-4438-f44e",
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
            "btc: 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
            "TRON TJCnKsPa7y5okkXvQAidZBzqx3QyQ6sxMW",
            "sol 7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtV",
            "rEb8TK3gBgk5auZkwc6sHnwrGVJH8DuaLh",
            "key 4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318",
        ] {
            assert!(wallet(w), "{w}");
            assert!(cleaned(w).is_err(), "{w}");
        }
        for talk in [
            "BTC 67000, ETH 1900, 1000PEPE pumping",
            "0x is a cool project",
            "long 0.5 BTC at 67,123.45 stop 66,800",
            "TP1 68000 TP2 69500 TP3 71000",
            "BitcoinToTheMoonAndBeyondForever",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "what a dump 😭😭😭",
            "Bitcoin is pumping today 5 guys and Solana too 3x",
            "67123 67500 68000 68500 69000 69500 70000 70500 71000 71500",
            "McDonald JavaScript iPhone 15 Pro Max 256GB",
            "RSI 14 at 26.11, MACD 12 26 9, EMA 200",
            "TP1 67,500 · TP2 68,200 · SL 66,100 · RR 2.4",
        ] {
            assert!(!wallet(talk), "{talk}");
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
        assert!(cleaned(&"x".repeat(2_000_000)).is_err(), "a huge text is turned away before it is worked through");
        assert_eq!(print("same"), print("same"));
        assert_ne!(print("same"), print("other"));
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
