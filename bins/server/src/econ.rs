//! The US economic calendar: what is due, when, and what it came to.
//!
//! Two sources. Nasdaq's calendar has the figures, a day at a time; it is free
//! and has no key, but it is unofficial, so whatever does not read as its answer
//! is rejected and the day tried again. The Fed's own page has the FOMC's dates,
//! well past the few weeks Nasdaq lists. Everything is held in memory, read
//! nearest day first in the background: a browser never waits on either, and a
//! restart reads it all again in seconds.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, bail};
use axum::extract::State;
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::time::sleep;
use tradoor_core::time::{days_from_civil, iso_date, new_york_behind, DAY, MINUTE};
use tradoor_store::now_ms;

use crate::api::AppState;
use crate::dns::Doh;

const NASDAQ: &str = "https://api.nasdaq.com/api/calendar/economicevents";
const FED: &str = "https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm";
/// Neither answers a client that does not look like a browser.
const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36";
const TIMEOUT: Duration = Duration::from_secs(15);

/// New York days held either side of today, and how far ahead the FOMC's are shown.
const BACK: i64 = 7;
const AHEAD: i64 = 21;
const FED_AHEAD: i64 = 60;

/// Nasdaq's API is unofficial: its days are asked for one at a time, spaced out.
const PAUSE: Duration = Duration::from_millis(400);
/// How often what is due is looked for.
const TICK: Duration = Duration::from_secs(15);
const HOUR: i64 = 60 * MINUTE;
/// Today, for a schedule that moves and figures that arrive.
const TODAY_MS: i64 = 5 * MINUTE;
/// Today again, while a release is out and its figure is not in.
const WAITING_MS: i64 = 45_000;
/// How long after its time a release's figure is waited for so closely.
const WAITED_MS: i64 = 20 * MINUTE;
/// A past day, until its figures settle at 03:00 New York the next morning; then never.
const SETTLING_MS: i64 = 30 * MINUTE;
const FUTURE_MS: i64 = 12 * HOUR;
const FED_MS: i64 = 24 * HOUR;
/// However due, not asked again sooner than this; one that failed, this much later.
const LEAST_MS: i64 = 30_000;
const FAILED_MS: i64 = 5 * MINUTE;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// When it is due, in ms; midnight New York where no time is given.
    at: i64,
    /// The New York date it falls on, `YYYY-MM-DD`.
    day: String,
    title: String,
    all_day: bool,
    /// CPI, PPI or the FOMC; otherwise other US data that moves markets.
    core: bool,
    actual: Option<String>,
    forecast: Option<String>,
    previous: Option<String>,
}

/// One New York day as last read.
#[derive(Default)]
struct Held {
    events: Vec<Event>,
    /// When it was last read whole; 0 for never.
    read: i64,
    tried: i64,
    failed: bool,
}

#[derive(Default)]
struct Book {
    days: HashMap<i64, Held>,
    fed: Vec<Event>,
    fed_read: i64,
    fed_tried: i64,
}

pub struct Calendar {
    http: reqwest::Client,
    book: RwLock<Book>,
}

/// The start of a New York date, as midnight UTC of that date.
fn new_york_today(now: i64) -> i64 {
    // Five hours back is the same New York date or the one the offset is read for.
    let guess = (now - 5 * HOUR).div_euclid(DAY) * DAY;
    (now - new_york_behind(guess) * MINUTE).div_euclid(DAY) * DAY
}

/// A New York wall-clock time on a date, in ms.
fn new_york_at(day: i64, minutes: i64) -> i64 {
    day + (minutes + new_york_behind(day)) * MINUTE
}

/// Text from a Nasdaq cell, entities decoded; empty and `-` are nothing.
fn clean(cell: &Value) -> Option<String> {
    let raw = match cell {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let text = raw.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty() && text != "-").then_some(text)
}

/// Whether a release is shown, and whether it is one of the three everyone waits
/// for. Nasdaq rates nothing, so the names are the filter: exact, so that `Core
/// CPI Index` and `Cleveland CPI` stay out.
fn weight(title: &str) -> Option<bool> {
    let t = title.to_lowercase();
    let core = ["cpi", "core cpi", "ppi", "core ppi", "fed interest rate decision", "fomc statement", "fomc press conference", "fomc economic projections", "fomc meeting minutes", "fomc minutes"];
    if core.contains(&t.as_str()) {
        return Some(true);
    }
    let major = ["nonfarm payrolls", "unemployment rate", "initial jobless claims", "jolts job openings", "ism manufacturing pmi", "ism non-manufacturing pmi", "ism services pmi", "retail sales", "core retail sales"];
    let gdp = t.strip_prefix("gdp").is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric()));
    let chair = t.strip_prefix("fed chair ").and_then(|rest| rest.strip_suffix(" speaks")).is_some_and(|name| !name.contains(' '));
    (major.contains(&t.as_str()) || gdp || chair || t.starts_with("pce price index") || t.starts_with("core pce price index")).then_some(false)
}

/// Nasdaq lists a release's monthly and yearly figures under one name and time:
/// `PPI` twice. A month's change is the smaller of the two, so that names them.
fn tell_apart(events: &mut [Event]) {
    let size = |e: &Event| e.previous.as_deref().filter(|p| p.ends_with('%')).and_then(|p| p.trim_end_matches('%').parse::<f64>().ok()).map(f64::abs);
    for i in 0..events.len() {
        for j in i + 1..events.len() {
            // Its capitals are not always the same twice: `PCE Price index`, `PCE price index`.
            if !events[i].title.eq_ignore_ascii_case(&events[j].title) || events[i].at != events[j].at {
                continue;
            }
            let (Some(a), Some(b)) = (size(&events[i]), size(&events[j])) else { continue };
            events[j].title = events[i].title.clone();
            let (month, year) = if a <= b { (i, j) } else { (j, i) };
            events[month].title.push_str(" m/m");
            events[year].title.push_str(" y/y");
        }
    }
}

/// The releases shown from one Nasdaq answer, or nothing where it is not one:
/// a day with no rows is a real, empty day.
fn nasdaq_day(day: i64, body: &Value) -> Option<Vec<Event>> {
    if body["status"]["rCode"].as_i64() != Some(200) {
        return None;
    }
    let mut out = Vec::new();
    for row in body["data"]["rows"].as_array().map_or(&[][..], Vec::as_slice) {
        if clean(&row["country"]).as_deref() != Some("United States") {
            continue;
        }
        let Some(title) = clean(&row["eventName"]) else { continue };
        let Some(core) = weight(&title) else { continue };
        // Despite its name, this column is New York time.
        let time = clean(&row["gmt"]).and_then(|t| {
            let (h, m) = t.split_once(':')?;
            Some(h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?)
        });
        out.push(Event {
            at: new_york_at(day, time.unwrap_or(0)),
            day: iso_date(day),
            title,
            all_day: time.is_none(),
            core,
            actual: clean(&row["actual"]),
            forecast: clean(&row["consensus"]),
            previous: clean(&row["previous"]),
        });
    }
    tell_apart(&mut out);
    Some(out)
}

/// Text of a piece of markup.
fn strip(html: &str) -> String {
    let (mut out, mut tag) = (String::new(), false);
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            c if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn month_of(name: &str) -> Option<i64> {
    let key = name.trim().to_lowercase();
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| key.starts_with(m)).map(|i| i as i64 + 1)
}

/// `16-17*`, `30-1` or `9`: the first day, the last, and whether it has a star.
fn meeting_days(text: &str) -> Option<(i64, i64, bool)> {
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).collect::<String>();
    let first = digits(text);
    let rest = text[first.len()..].trim_start();
    let first = first.parse::<i64>().ok()?;
    let (last, rest) = match rest.strip_prefix('-') {
        Some(after) => {
            let after = after.trim_start();
            let last = digits(after);
            (last.parse::<i64>().ok()?, after[last.len()..].trim_start())
        }
        None => (first, rest),
    };
    Some((first, last, rest.starts_with('*')))
}

/// The FOMC's meetings on the Fed's page, as the day each decides and whether it
/// brings projections. Under each `<year> FOMC Meetings`, rows of a month (`June`,
/// or `Apr/May` when one straddles two) and days. Notation votes and unscheduled
/// calls are left out: no statement at two, no press conference.
fn fomc_meetings(html: &str) -> Vec<(i64, bool)> {
    const HEAD: &str = " FOMC Meetings";
    let heads: Vec<(usize, i64)> = html.match_indices(HEAD).filter_map(|(at, _)| Some((at, html.get(at.checked_sub(4)?..at)?.parse().ok()?))).collect();
    let mut out = Vec::new();
    for (n, (at, year)) in heads.iter().enumerate() {
        let mut rest = &html[at + HEAD.len()..heads.get(n + 1).map_or(html.len(), |h| h.0)];
        while let Some(found) = rest.find("fomc-meeting__month") {
            rest = &rest[found..];
            let Some((label, after)) = between(rest, "<strong>", "</strong>") else { break };
            let Some(date) = after.find("fomc-meeting__date") else { break };
            let after = &after[date..];
            let Some((days, tail)) = between(after, ">", "</div>") else { break };
            rest = tail;
            let days = strip(days);
            let label = strip(label);
            let lower = days.to_lowercase();
            if ["notation", "unscheduled", "cancel"].iter().any(|w| lower.contains(w)) {
                continue;
            }
            let Some((first, last, star)) = meeting_days(&days) else { continue };
            let (start_name, end_name) = label.split_once('/').map_or((label.as_str(), None), |(a, b)| (a, Some(b)));
            let Some(start) = month_of(start_name) else { continue };
            let Some(mut end) = end_name.map_or(Some(start), month_of) else { continue };
            let mut y = *year;
            if end_name.is_none() && last < first {
                end = start + 1;
            }
            if end < start {
                y += 1;
            }
            if end > 12 {
                end -= 12;
                y += 1;
            }
            out.push((days_from_civil(y, end as u32, last as u32) * DAY, star));
        }
    }
    out
}

/// What lies between the first `open` and the next `close`, and what follows it.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<(&'a str, &'a str)> {
    let from = text.find(open)? + open.len();
    let to = text[from..].find(close)? + from;
    Some((&text[from..to], &text[to + close.len()..]))
}

/// A meeting's decision, projections and press conference, at their usual two and half past.
fn fomc_events(meetings: &[(i64, bool)]) -> Vec<Event> {
    let event = |day: i64, title: &str, minutes: i64| Event { at: new_york_at(day, minutes), day: iso_date(day), title: title.to_string(), all_day: false, core: true, actual: None, forecast: None, previous: None };
    let mut out = Vec::new();
    for (day, projections) in meetings {
        out.push(event(*day, "Fed Interest Rate Decision", 14 * 60));
        if *projections {
            out.push(event(*day, "FOMC Economic Projections", 14 * 60));
        }
        out.push(event(*day, "FOMC Press Conference", 14 * 60 + 30));
    }
    out
}

/// Whether a day should be read again.
fn due(day: i64, held: Option<&Held>, today: i64, now: i64) -> bool {
    let Some(held) = held else { return true };
    if now - held.tried < if held.failed { FAILED_MS } else { LEAST_MS } {
        return false;
    }
    if held.read == 0 {
        return true;
    }
    let age = now - held.read;
    if day < today {
        return held.read < new_york_at(day + DAY, 3 * 60) && age > SETTLING_MS;
    }
    if day > today {
        return age > FUTURE_MS;
    }
    // A release with a figure to come: one out, and not in. Speeches have none to wait for.
    let waiting = held.events.iter().any(|e| !e.all_day && e.at <= now && now - e.at < WAITED_MS && e.actual.is_none() && (e.forecast.is_some() || e.previous.is_some()));
    age > if waiting { WAITING_MS } else { TODAY_MS }
}

/// What is shown: Nasdaq's days, and the Fed's dates where Nasdaq has no FOMC row
/// that day, for Nasdaq's carry the figures.
fn merged(book: &Book, today: i64) -> Vec<Event> {
    let window = |day: i64, ahead: i64| day >= today - BACK * DAY && day <= today + ahead * DAY;
    let mut events: Vec<Event> = book.days.iter().filter(|(day, _)| window(**day, AHEAD)).flat_map(|(_, held)| held.events.iter().cloned()).collect();
    let fomc = |e: &Event| {
        let t = e.title.to_lowercase();
        t.contains("fomc") || t.contains("fed interest rate decision")
    };
    let covered: HashSet<String> = events.iter().filter(|e| fomc(e)).map(|e| e.day.clone()).collect();
    events.extend(book.fed.iter().filter(|e| window(new_york_today(e.at), FED_AHEAD) && !covered.contains(&e.day)).cloned());
    events.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.title.cmp(&b.title)));
    events
}

impl Calendar {
    /// Starts reading, for as long as the server runs.
    pub fn start(doh: &Arc<Doh>) -> anyhow::Result<Arc<Self>> {
        let http = reqwest::Client::builder().dns_resolver(doh.resolver()).user_agent(BROWSER).timeout(TIMEOUT).build()?;
        let it = Arc::new(Self { http, book: RwLock::new(Book::default()) });
        tokio::spawn(it.clone().keep());
        Ok(it)
    }

    async fn keep(self: Arc<Self>) {
        loop {
            let now = now_ms();
            for (i, day) in self.due_days(now).into_iter().enumerate() {
                if i > 0 {
                    sleep(PAUSE).await;
                }
                self.read_day(day).await;
            }
            if self.fed_due(now) {
                self.read_fed().await;
            }
            sleep(TICK).await;
        }
    }

    /// The days to read, today first, then outward, the past before the future.
    fn due_days(&self, now: i64) -> Vec<i64> {
        let today = new_york_today(now);
        let mut book = self.book.write().unwrap_or_else(|e| e.into_inner());
        book.days.retain(|day, _| *day >= today - BACK * DAY && *day <= today + AHEAD * DAY);
        let mut days: Vec<i64> = (-BACK..=AHEAD).map(|k| today + k * DAY).filter(|day| due(*day, book.days.get(day), today, now)).collect();
        days.sort_by_key(|day| {
            let k = (day - today) / DAY;
            k.abs() * 2 + i64::from(k > 0)
        });
        days
    }

    fn fed_due(&self, now: i64) -> bool {
        let book = self.book.read().unwrap_or_else(|e| e.into_inner());
        now - book.fed_tried >= FAILED_MS && (book.fed_read == 0 || now - book.fed_read > FED_MS)
    }

    async fn read_day(&self, day: i64) {
        // Nasdaq files each US day's events under the following date.
        let asked = async {
            let response = self.http.get(format!("{NASDAQ}?date={}", iso_date(day + DAY))).header("accept", "application/json, text/plain, */*").header("origin", "https://www.nasdaq.com").header("referer", "https://www.nasdaq.com/").send().await?.error_for_status()?;
            let body: Value = serde_json::from_slice(&response.bytes().await?)?;
            nasdaq_day(day, &body).ok_or_else(|| anyhow!("not an answer of Nasdaq's"))
        };
        let result: anyhow::Result<Vec<Event>> = asked.await;
        let now = now_ms();
        let mut book = self.book.write().unwrap_or_else(|e| e.into_inner());
        let held = book.days.entry(day).or_default();
        held.tried = now;
        match result {
            Ok(events) => {
                held.events = events;
                held.read = now;
                held.failed = false;
            }
            // What was read before stands until a reading replaces it.
            Err(e) => {
                held.failed = true;
                tracing::warn!(day = %iso_date(day), error = %e, "reading the economic calendar");
            }
        }
    }

    async fn read_fed(&self) {
        let asked = async {
            let html = self.http.get(FED).header("accept", "text/html").send().await?.error_for_status()?.text().await?;
            let meetings = fomc_meetings(&html);
            // A new layout would read as no meetings: that must not wipe the dates held.
            if meetings.is_empty() {
                bail!("no FOMC meetings on the Fed's page");
            }
            anyhow::Ok(fomc_events(&meetings))
        };
        let result = asked.await;
        let now = now_ms();
        let mut book = self.book.write().unwrap_or_else(|e| e.into_inner());
        book.fed_tried = now;
        match result {
            Ok(events) => {
                book.fed = events;
                book.fed_read = now;
            }
            Err(e) => tracing::warn!(error = %e, "reading the FOMC's calendar"),
        }
    }

    fn answer(&self, now: i64) -> Value {
        let today = new_york_today(now);
        let book = self.book.read().unwrap_or_else(|e| e.into_inner());
        let unread = (-BACK..=AHEAD).filter(|k| book.days.get(&(today + k * DAY)).is_none_or(|held| held.read == 0)).count() + usize::from(book.fed_read == 0);
        json!({ "events": merged(&book, today), "pending": unread })
    }
}

/// `GET /api/econ`: the releases around today, and how many days are still to be read.
pub async fn calendar(State(state): State<AppState>) -> Json<Value> {
    Json(state.econ.answer(now_ms()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradoor_core::time::parse_iso;

    fn day(iso: &str) -> i64 {
        parse_iso(iso).unwrap()
    }

    fn row(name: &str, gmt: &str, actual: &str, consensus: &str, previous: &str) -> Value {
        json!({ "gmt": gmt, "country": "United States", "eventName": name, "actual": actual, "consensus": consensus, "previous": previous })
    }

    fn answer(rows: Vec<Value>) -> Value {
        json!({ "status": { "rCode": 200 }, "data": { "rows": rows } })
    }

    #[test]
    fn the_releases_that_matter_are_picked_by_their_exact_names() {
        for (name, core) in [("CPI", true), ("Core PPI", true), ("Fed Interest Rate Decision", true), ("Nonfarm Payrolls", false), ("GDP Annualized QoQ", false), ("Fed Chair Powell Speaks", false), ("Core PCE Price Index MoM", false)] {
            assert_eq!(weight(name), Some(core), "{name}");
        }
        for name in ["Core CPI Index", "Cleveland CPI", "Atlanta Fed GDPNow", "GDPNow", "Fed Goolsbee Speaks", "FOMC Member Daly Speaks", "Crude Oil Inventories"] {
            assert_eq!(weight(name), None, "{name}");
        }
    }

    #[test]
    fn a_nasdaq_day_reads_new_york_time_and_cleans_its_cells() {
        let body = answer(vec![row("CPI", "08:30", "0.4%", " ", "&nbsp;"), row("Crude Oil Inventories", "10:30", "1M", "", ""), row("Core CPI", "All Day", "&nbsp;", "0.2%", "-")]);
        let summer = nasdaq_day(day("2026-07-14"), &body).unwrap();
        assert_eq!(summer.len(), 2, "what is not followed is left out");
        assert_eq!(summer[0].at, parse_iso("2026-07-14T12:30:00Z").unwrap(), "four hours behind in July");
        assert_eq!((summer[0].actual.as_deref(), summer[0].forecast.as_deref(), summer[0].previous.as_deref()), (Some("0.4%"), None, None));
        assert!(summer[1].all_day && summer[1].actual.is_none() && summer[1].previous.is_none());
        let winter = nasdaq_day(day("2026-12-10"), &body).unwrap();
        assert_eq!(winter[0].at, parse_iso("2026-12-10T13:30:00Z").unwrap(), "five in December");
        assert_eq!(winter[0].day, "2026-12-10");
    }

    #[test]
    fn what_is_not_nasdaqs_answer_is_refused_but_an_empty_day_is_a_day() {
        assert_eq!(nasdaq_day(0, &json!({ "status": { "rCode": 400 } })), None);
        assert_eq!(nasdaq_day(0, &json!("Access denied")), None);
        assert_eq!(nasdaq_day(0, &json!({ "status": { "rCode": 200 }, "data": null })), Some(Vec::new()));
    }

    #[test]
    fn a_month_and_a_year_under_one_name_are_told_apart() {
        let events = nasdaq_day(day("2026-11-13"), &answer(vec![row("PPI", "08:30", "", "0.4%", "1.3%"), row("PPI", "08:30", "", "", "6.3%"), row("Core PPI", "08:30", "", "", "5.0%"), row("Core PPI", "08:30", "", "", "0.7%"), row("CPI", "08:30", "", "", "0.6%")])).unwrap();
        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["PPI m/m", "PPI y/y", "Core PPI y/y", "Core PPI m/m", "CPI"]);
        let cases = nasdaq_day(day("2026-11-25"), &answer(vec![row("PCE Price index", "08:30", "", "", "3.6%"), row("PCE price index", "08:30", "", "", "0.5%")])).unwrap();
        assert_eq!(cases.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["PCE Price index y/y", "PCE Price index m/m"]);
    }

    const FED_PAGE: &str = r#"<h4>2026 FOMC Meetings</h4>
        <div class="fomc-meeting__month"><strong>June</strong></div><div class="fomc-meeting__date">16-17*</div>
        <div class="fomc-meeting__month"><strong>Apr/May</strong></div><div class="fomc-meeting__date">28-29</div>
        <div class="fomc-meeting__month"><strong>July</strong></div><div class="fomc-meeting__date">30-1</div>
        <div class="fomc-meeting__month"><strong>August</strong></div><div class="fomc-meeting__date">5 (notation vote)</div>
        <div class="fomc-meeting__month"><strong>December</strong></div><div class="fomc-meeting__date">8-9<br/>&nbsp;</div>
        <h4>2027 FOMC Meetings</h4>
        <div class="fomc-meeting__month"><strong>Dec/Jan</strong></div><div class="fomc-meeting__date">30-1</div>"#;

    #[test]
    fn the_feds_page_gives_the_day_each_meeting_decides() {
        let meetings = fomc_meetings(FED_PAGE);
        let read: Vec<(String, bool)> = meetings.iter().map(|(d, star)| (iso_date(*d), *star)).collect();
        let want: Vec<(String, bool)> = [("2026-06-17", true), ("2026-05-29", false), ("2026-08-01", false), ("2026-12-09", false), ("2028-01-01", false)].iter().map(|(d, s)| ((*d).to_string(), *s)).collect();
        assert_eq!(read, want);
        let events = fomc_events(&meetings[..1]);
        assert_eq!(events.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Fed Interest Rate Decision", "FOMC Economic Projections", "FOMC Press Conference"]);
        assert_eq!(events[0].at, parse_iso("2026-06-17T18:00:00Z").unwrap());
        assert!(fomc_meetings("<html>a new layout</html>").is_empty());
    }

    #[test]
    fn nasdaqs_fomc_rows_stand_over_the_feds_dates_that_day() {
        let (nasdaq, plain) = (day("2026-06-17"), day("2026-07-29"));
        let mut book = Book { fed: fomc_events(&[(nasdaq, false), (plain, false), (day("2027-03-17"), false)]), ..Book::default() };
        book.days.insert(nasdaq, Held { events: nasdaq_day(nasdaq, &answer(vec![row("Fed Interest Rate Decision", "14:00", "3.75%", "3.75%", "3.75%")])).unwrap(), read: 1, ..Held::default() });
        let events = merged(&book, nasdaq);
        assert_eq!(events.iter().map(|e| (e.day.as_str(), e.actual.is_some())).collect::<Vec<_>>(), [("2026-06-17", true), ("2026-07-29", false), ("2026-07-29", false)], "the one a year off is past the window");
    }

    #[test]
    fn a_day_is_read_again_as_often_as_it_can_have_changed() {
        let today = day("2026-11-11");
        let now = new_york_at(today, 9 * 60);
        let held = |ago: i64, events: Vec<Event>| Held { events, read: now - ago, tried: now - ago, failed: false };
        assert!(due(today, None, today, now), "never read");
        assert!(!due(today, Some(&held(4 * MINUTE, Vec::new())), today, now));
        assert!(due(today, Some(&held(6 * MINUTE, Vec::new())), today, now));
        let released = |forecast: Option<&str>, actual: Option<&str>| Event { at: new_york_at(today, 8 * 60 + 50), day: iso_date(today), title: "CPI".into(), all_day: false, core: true, actual: actual.map(Into::into), forecast: forecast.map(Into::into), previous: None };
        assert!(due(today, Some(&held(MINUTE, vec![released(Some("0.3%"), None)])), today, now), "out, and its figure not in");
        assert!(!due(today, Some(&held(MINUTE, vec![released(Some("0.3%"), Some("0.4%"))])), today, now), "its figure in");
        assert!(!due(today, Some(&held(MINUTE, vec![released(None, None)])), today, now), "a speech has no figure to wait for");
        assert!(!due(today + DAY, Some(&held(HOUR, Vec::new())), today, now));
        assert!(due(today + DAY, Some(&held(13 * HOUR, Vec::new())), today, now));
        assert!(!due(today - 3 * DAY, Some(&held(20 * HOUR, Vec::new())), today, now), "settled: never again");
        let failed = Held { read: 0, tried: now - MINUTE, failed: true, ..Held::default() };
        assert!(!due(today, Some(&failed), today, now), "a day that failed waits its turn");
    }

    #[test]
    fn new_yorks_date_is_its_own_not_utcs() {
        assert_eq!(iso_date(new_york_today(parse_iso("2026-11-11T03:00:00Z").unwrap())), "2026-11-10", "ten at night there");
        assert_eq!(iso_date(new_york_today(parse_iso("2026-11-11T06:00:00Z").unwrap())), "2026-11-11");
        assert_eq!(iso_date(new_york_today(parse_iso("2026-07-11T03:30:00Z").unwrap())), "2026-07-10");
        assert_eq!(iso_date(new_york_today(parse_iso("2026-07-11T04:30:00Z").unwrap())), "2026-07-11");
    }
}
