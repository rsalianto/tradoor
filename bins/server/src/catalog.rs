//! The market catalogue a symbol search offers: every tradable spot and
//! perpetual market on Binance, Bybit, OKX, Bitget, Hyperliquid (trade.xyz's
//! stocks and indices with its coins), Kraken, Lighter, Gate, Aster, HTX,
//! Bitfinex, Bitstamp, Deribit and Coinbase (spot alone), with 24h stats, and each
//! exchange's own name for it where that is not this app's — and the series
//! this server works out itself (`indices`).
//!
//! Fetched here rather than by each browser because the raw lists are several
//! megabytes; the browser gets one compact, compressed response it can search
//! locally without a request per keystroke. That response (`wire`) lists each
//! venue's markets as rows of a few columns, rounded to what is shown, and is
//! compressed with brotli once per refresh rather than per request: some
//! 200 KB, a fraction of the plain list. The list is read
//! again in the background before it goes stale, so no request waits on the
//! exchanges for it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use serde::Serialize;
use serde_json::json;
use tokio::sync::Mutex;
use tradoor_venues::{aster, binance, bitfinex, bitget, bitstamp, bybit, coinbase, deribit, gate, htx, hyperliquid, kraken, lighter, okx, Exchange, Product, Venue};

const TTL: Duration = Duration::from_secs(30 * 60);

/// A symbol less its quote, as the page's `coinOf` has it: `BTCUSDT` → `BTC`.
fn coin_of(symbol: &str) -> &str {
    ["USDT", "USDC", "FDUSD", "BUSD", "USD"].iter().find_map(|q| symbol.strip_suffix(q)).filter(|c| !c.is_empty()).unwrap_or(symbol)
}

pub struct Catalog {
    http: reqwest::Client,
    binance_spot_rest: String,
    state: Mutex<Option<Cached>>,
    /// Held while the lists are read: one refresh at a time, and those that
    /// ask meanwhile take its result.
    refreshing: Mutex<()>,
    /// The series this server works out, listed under the `index` venue.
    indices: std::sync::OnceLock<Arc<crate::indices::Indices>>,
}

struct Cached {
    at: Instant,
    body: Arc<Bytes>,
    /// The compact list, as JSON and brotli-compressed.
    wire: Wire,
    by_venue: Vec<(Venue, Vec<Product>)>,
}

/// The catalogue as `/api/catalog` sends it: plain, and compressed once.
#[derive(Clone)]
pub struct Wire {
    pub json: Bytes,
    pub brotli: Bytes,
}

/// The columns of a `wire` row, in order; a row stops at its last non-null.
const COLUMNS: [&str; 9] = ["symbol", "base", "quote", "priceDecimals", "quoteVolume24h", "lastPrice", "changePct", "native", "name"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VenueStatus {
    id: &'static str,
    label: &'static str,
    count: usize,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Response<'a> {
    updated_at: i64,
    venues: Vec<&'a VenueStatus>,
    products: Vec<&'a Product>,
}

impl Catalog {
    pub fn new(binance_spot_rest: String, doh: &std::sync::Arc<crate::dns::Doh>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().dns_resolver(doh.resolver()).user_agent(concat!("tradoor/", env!("CARGO_PKG_VERSION"))).timeout(Duration::from_secs(30)).build()?;
        Ok(Self { http, binance_spot_rest, state: Mutex::new(None), refreshing: Mutex::new(()), indices: std::sync::OnceLock::new() })
    }

    /// Reads the lists now and again before each goes stale, for as long as
    /// the server runs: requests are answered from what was read.
    pub fn keep_fresh(self: &Arc<Self>) {
        let catalog = self.clone();
        tokio::spawn(async move {
            loop {
                catalog.refresh(true).await;
                tokio::time::sleep(TTL - Duration::from_secs(120)).await;
            }
        });
    }

    /// The compact catalogue `/api/catalog` sends.
    pub async fn wire(&self) -> Wire {
        self.get().await;
        let state = self.state.lock().await;
        state.as_ref().map(|c| c.wire.clone()).expect("read by get")
    }

    /// Lists the series this server works out, from the next refresh on.
    pub fn list_indices(&self, indices: Arc<crate::indices::Indices>) {
        let _ = self.indices.set(indices);
    }

    async fn text(&self, url: &str) -> Result<String, String> {
        let resp = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("{status}: {}", body.chars().take(200).collect::<String>()));
        }
        Ok(body)
    }

    async fn post(&self, url: &str, body: &'static str) -> Result<String, String> {
        let resp = self.http.post(url).header("content-type", "application/json").body(body).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("{status}: {}", text.chars().take(200).collect::<String>()));
        }
        Ok(text)
    }

    /// A list and, when it answers, its ticker table.
    async fn pair(&self, list: String, tickers: String) -> Result<(String, Option<String>), String> {
        let (list, tickers) = tokio::join!(self.text(&list), self.text(&tickers));
        Ok((list?, tickers.ok()))
    }

    async fn fetch(&self, venue: Venue) -> Result<Vec<Product>, String> {
        let err = |e: tradoor_venues::VenueError| e.to_string();
        match venue.exchange() {
            Exchange::Okx => {
                let (list, tickers) = self.pair(okx::instruments_url(venue), okx::tickers_url(venue)).await?;
                okx::parse_products(venue, &list, tickers.as_deref()).map_err(err)
            }
            Exchange::Bitget => {
                let (list, tickers) = self.pair(bitget::instruments_url(venue), bitget::tickers_url(venue)).await?;
                bitget::parse_products(venue, &list, tickers.as_deref()).map_err(err)
            }
            Exchange::Hyperliquid => {
                let (coins, xyz) = tokio::join!(self.post(hyperliquid::INFO, hyperliquid::META_AND_CTXS), self.post(hyperliquid::INFO, hyperliquid::XYZ_META_AND_CTXS));
                hyperliquid::parse_products(&coins?, xyz.ok().as_deref()).map_err(err)
            }
            Exchange::Index => Ok(match self.indices.get() {
                Some(indices) => indices.products().await,
                None => Vec::new(),
            }),
            Exchange::Kraken if venue == Venue::KrakenSpot => {
                let (list, tickers) = self.pair(kraken::spot_pairs_url(), kraken::spot_tickers_url()).await?;
                kraken::parse_spot(&list, tickers.as_deref()).map_err(err)
            }
            Exchange::Kraken => {
                let (list, tickers) = self.pair(kraken::futures_instruments_url(), kraken::futures_tickers_url()).await?;
                kraken::parse_futures(&list, tickers.as_deref()).map_err(err)
            }
            Exchange::Lighter => lighter::parse_products(&self.text(&lighter::details_url()).await?).map_err(err),
            Exchange::Gate if venue == Venue::GateSpot => {
                let (list, tickers) = self.pair(gate::spot_pairs_url(), gate::spot_tickers_url()).await?;
                gate::parse_spot(&list, tickers.as_deref()).map_err(err)
            }
            Exchange::Gate => {
                let (list, tickers) = self.pair(gate::contracts_url(), gate::perp_tickers_url()).await?;
                gate::parse_perps(&list, tickers.as_deref()).map_err(err)
            }
            Exchange::Aster => {
                let (list, tickers) = self.pair(aster::exchange_info_url(venue), aster::tickers_url(venue)).await?;
                binance::parse_products(venue, &list, tickers.as_deref()).map_err(err)
            }
            Exchange::Htx if venue == Venue::HtxSpot => {
                let (list, tickers) = self.pair(htx::spot_symbols_url(), htx::spot_tickers_url()).await?;
                htx::parse_spot(&list, tickers.as_deref()).map_err(err)
            }
            Exchange::Htx => {
                let (list, tickers) = self.pair(htx::contracts_url(), htx::perp_tickers_url()).await?;
                htx::parse_perps(&list, tickers.as_deref()).map_err(err)
            }
            Exchange::Bitfinex => {
                let (list, tickers) = self.pair(bitfinex::pairs_url(venue), bitfinex::tickers_url()).await?;
                bitfinex::parse_products(venue, &list, tickers.as_deref()).map_err(err)
            }
            Exchange::Deribit => {
                let (instruments, btc, eth, usdc) = (deribit::instruments_url(), deribit::summary_url("BTC"), deribit::summary_url("ETH"), deribit::summary_url("USDC"));
                let (list, btc, eth, usdc) = tokio::join!(self.text(&instruments), self.text(&btc), self.text(&eth), self.text(&usdc));
                deribit::parse_perps(&list?, &[btc.ok(), eth.ok(), usdc.ok()]).map_err(err)
            }
            Exchange::Bitstamp => {
                let (list, tickers) = self.pair(bitstamp::markets_url(), bitstamp::tickers_url()).await?;
                bitstamp::parse_products(venue, &list, tickers.as_deref()).map_err(err)
            }
            Exchange::Coinbase => {
                let (list, stats) = self.pair(coinbase::products_url(), coinbase::stats_url()).await?;
                coinbase::parse_spot(&list, stats.as_deref()).map_err(err)
            }
            Exchange::Binance => {
                let base = if venue == Venue::BinanceSpot { self.binance_spot_rest.as_str() } else { binance::PERP_REST };
                let (info_url, tickers_url) = (binance::exchange_info_url(base, venue), binance::tickers_url(base, venue));
                let (info, tickers) = tokio::join!(self.text(&info_url), self.text(&tickers_url));
                binance::parse_products(venue, &info?, tickers.ok().as_deref()).map_err(|e| e.to_string())
            }
            Exchange::Bybit => {
                let tickers = self.text(&bybit::tickers_url(bybit::REST, venue)).await.ok();
                let mut out = Vec::new();
                let mut cursor: Option<String> = None;
                for _ in 0..20 {
                    let page = self.text(&bybit::instruments_url(bybit::REST, venue, cursor.as_deref())).await?;
                    let (products, next) = bybit::parse_instruments(venue, &page, tickers.as_deref()).map_err(|e| e.to_string())?;
                    out.extend(products);
                    match next {
                        Some(c) => cursor = Some(c),
                        None => break,
                    }
                }
                Ok(out)
            }
        }
    }

    /// The exchange's own names for these markets of one venue, as
    /// `ours -> theirs`. OKX calls ours `BTC-USDT-SWAP` and Hyperliquid calls
    /// it `BTC`; a stream has to be asked in the exchange's words and answers
    /// in them too. A market the catalogue does not know keeps its own name.
    pub async fn native_names(&self, venue: Venue, symbols: &[String]) -> HashMap<String, String> {
        self.get().await;
        let state = self.state.lock().await;
        let listed = state.as_ref().and_then(|c| c.by_venue.iter().find(|(v, _)| *v == venue).map(|(_, p)| p));
        symbols
            .iter()
            .map(|symbol| {
                let native = listed
                    .and_then(|products| products.iter().find(|p| p.symbol == *symbol))
                    .and_then(|p| p.native.clone())
                    .unwrap_or_else(|| symbol.clone());
                (symbol.clone(), native)
            })
            .collect()
    }

    /// Those of `markets` (`bybit-perp:BTCUSDT`) an exchange lists, in their order.
    pub async fn listed(&self, markets: &[String]) -> Vec<String> {
        self.get().await;
        let state = self.state.lock().await;
        let Some(cached) = state.as_ref() else { return Vec::new() };
        markets.iter().filter(|m| cached.by_venue.iter().flat_map(|(_, p)| p).any(|p| p.market == **m)).cloned().collect()
    }

    /// The `n` coins with the most turnover over a day on Binance's and Bybit's
    /// USDT perpetuals, one market each: Binance's where it lists the coin, else
    /// Bybit's. Stablecoins are left out; they have nothing to chart.
    pub async fn perp_leaders(&self, n: usize) -> Vec<String> {
        const STABLE: [&str; 12] = ["USDC", "FDUSD", "TUSD", "USDP", "DAI", "BUSD", "USDE", "USD1", "RLUSD", "PYUSD", "USDS", "XUSD"];
        self.get().await;
        let state = self.state.lock().await;
        let Some(cached) = state.as_ref() else { return Vec::new() };
        let mut coins: HashMap<&str, (f64, Option<&str>, Option<&str>)> = HashMap::new();
        for (venue, products) in &cached.by_venue {
            if !matches!(venue, Venue::BinancePerp | Venue::BybitPerp) {
                continue;
            }
            for p in products.iter().filter(|p| p.kind == "perp" && p.quote == "USDT" && !STABLE.contains(&p.base.as_str())) {
                let coin = coins.entry(p.base.as_str()).or_default();
                coin.0 += p.quote_volume_24h.unwrap_or(0.0);
                if *venue == Venue::BinancePerp {
                    coin.1 = Some(&p.market);
                } else {
                    coin.2 = Some(&p.market);
                }
            }
        }
        let mut ranked: Vec<_> = coins.into_values().filter(|(turnover, _, _)| *turnover > 0.0).collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked.into_iter().filter_map(|(_, binance, bybit)| binance.or(bybit).map(str::to_string)).take(n).collect()
    }

    /// Whether any exchange lists a perpetual on `coin` — the coin as a chart
    /// names it, its symbol less the quote: `BTC`, `1000PEPE`.
    pub async fn lists_perp(&self, coin: &str) -> bool {
        self.get().await;
        let state = self.state.lock().await;
        state.as_ref().is_some_and(|c| c.by_venue.iter().flat_map(|(_, p)| p).any(|p| p.kind == "perp" && (p.base == coin || coin_of(&p.symbol) == coin)))
    }

    /// The serialised catalogue, read again when older than the TTL — which
    /// `keep_fresh` sees to first.
    pub async fn get(&self) -> Arc<Bytes> {
        if let Some(cached) = self.state.lock().await.as_ref().filter(|c| c.at.elapsed() < TTL) {
            return cached.body.clone();
        }
        self.refresh(false).await
    }

    /// Every venue's list read again, without the catalogue locked while the
    /// exchanges answer. A venue that fails keeps its previous list: stale
    /// names beat an empty search box. Unless `force`d, a list another
    /// refresh has just read is taken as it is.
    async fn refresh(&self, force: bool) -> Arc<Bytes> {
        let _one = self.refreshing.lock().await;
        if !force {
            if let Some(cached) = self.state.lock().await.as_ref().filter(|c| c.at.elapsed() < TTL) {
                return cached.body.clone();
            }
        }
        let results = futures_util::future::join_all(Venue::ALL.iter().map(|v| self.fetch(*v))).await;
        let previous = self.state.lock().await.as_ref().map(|c| c.by_venue.clone()).unwrap_or_default();
        let mut by_venue = Vec::new();
        let mut venues = Vec::new();
        for (venue, result) in Venue::ALL.into_iter().zip(results) {
            let (products, error) = match result {
                Ok(p) => (p, None),
                Err(e) => {
                    tracing::warn!(venue = venue.id(), error = %e, "catalogue fetch failed");
                    (previous.iter().find(|(v, _)| *v == venue).map(|(_, p)| p.clone()).unwrap_or_default(), Some(e))
                }
            };
            venues.push(VenueStatus { id: venue.id(), label: venue.label(), count: products.len(), error });
            by_venue.push((venue, products));
        }
        let updated_at = tradoor_store::now_ms();
        let response = Response { updated_at, venues: venues.iter().collect(), products: by_venue.iter().flat_map(|(_, p)| p).collect() };
        let body = Arc::new(Bytes::from(serde_json::to_vec(&response).expect("catalogue serialises")));
        let json = Bytes::from(serde_json::to_vec(&wire_of(updated_at, &venues, &by_venue)).expect("catalogue serialises"));
        // The best brotli does, once per refresh: a few seconds off the runtime's threads.
        let plain = json.clone();
        let brotli = tokio::task::spawn_blocking(move || compress(&plain)).await.unwrap_or_default();
        *self.state.lock().await = Some(Cached { at: Instant::now(), body: body.clone(), wire: Wire { json, brotli: Bytes::from(brotli) }, by_venue });
        body
    }
}

/// The compact catalogue: each venue's markets as rows of `COLUMNS`, the
/// turnover to four figures and the change to two decimals, as shown.
fn wire_of(updated_at: i64, venues: &[VenueStatus], by_venue: &[(Venue, Vec<Product>)]) -> serde_json::Value {
    let figures = |v: f64| {
        let scale = 10f64.powi(3 - v.abs().log10().floor() as i32);
        if v == 0.0 || !v.is_finite() { v } else { (v * scale).round() / scale }
    };
    let markets: serde_json::Map<String, serde_json::Value> = by_venue
        .iter()
        .map(|(venue, products)| {
            let rows = products
                .iter()
                .map(|p| {
                    let mut row = vec![
                        json!(p.symbol),
                        json!(p.base),
                        json!(p.quote),
                        json!(p.price_decimals),
                        json!(p.quote_volume_24h.map(figures)),
                        json!(p.last_price),
                        json!(p.change_pct.map(|c| (c * 100.0).round() / 100.0)),
                        json!(p.native),
                        json!(p.name),
                    ];
                    while row.last().is_some_and(serde_json::Value::is_null) {
                        row.pop();
                    }
                    serde_json::Value::Array(row)
                })
                .collect();
            (venue.id().to_string(), serde_json::Value::Array(rows))
        })
        .collect();
    json!({ "updatedAt": updated_at, "venues": venues, "columns": COLUMNS, "markets": markets })
}

fn compress(plain: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(plain.len() / 4);
    let params = brotli::enc::BrotliEncoderParams { quality: 11, lgwin: 22, ..Default::default() };
    let _ = brotli::BrotliCompress(&mut std::io::Cursor::new(plain), &mut out, &params);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_coin_is_its_symbol_less_the_quote() {
        assert_eq!(super::coin_of("BTCUSDT"), "BTC");
        assert_eq!(super::coin_of("1000PEPEUSDC"), "1000PEPE");
        assert_eq!(super::coin_of("ETHUSD"), "ETH");
        assert_eq!(super::coin_of("USDT"), "USDT", "a bare quote stays itself");
    }
}
