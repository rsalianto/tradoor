//! tradoor-server — the small API next to the static files.
//!
//! It serves the market catalogue, drawings and workspaces, and (in development)
//! the candle partitions and the built web app. Market data does not normally
//! pass through it: the browser streams from the exchanges directly and reads
//! history as static files, which nginx and the CDN serve in production. The one
//! exception is the relay (`relay.rs`), which a browser falls back to only when
//! its network blocks the exchanges' names.

mod account;
mod alert_watch;
mod alerts;
mod api;
mod catalog;
mod dns;
mod drives;
mod drawings;
mod econ;
mod favourites;
mod flow;
mod footprint;
mod footprint_history;
mod import;
mod indices;
mod journal;
mod limits;
mod liquidations;
mod open_interest;
mod presence;
mod relay;
mod research;
mod session;
mod signals;
mod telegram;
mod trades;
mod training;
mod workspaces;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tradoor_store::{DataDir, Db};
use tradoor_venues::binance;

#[derive(Parser)]
#[command(about = "tradoor API server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, env = "DATABASE_URL", global = true, hide_env_values = true)]
    database_url: Option<String>,
    #[arg(long, env = "TRADOOR_DATA_DIR", default_value = "./data", global = true)]
    data_dir: PathBuf,
    #[arg(long, env = "TRADOOR_BIND", default_value = "127.0.0.1:8080")]
    bind: SocketAddr,
    /// Serve the built web app from here as well (development and single-box setups).
    #[arg(long, env = "TRADOOR_WEB_DIR")]
    web_dir: Option<PathBuf>,
    #[arg(long, env = "BINANCE_SPOT_REST", default_value = binance::SPOT_REST)]
    binance_spot_rest: String,
    /// Tickers whose liquidations are kept for 24 hours, gathered from every
    /// perpetual exchange. An empty list turns the collection off.
    #[arg(long, env = "TRADOOR_LIQUIDATIONS", default_value = "BTCUSDT,ETHUSDT,SOLUSDT", value_delimiter = ',')]
    liquidations: Vec<String>,
    /// Coins whose open interest is recorded from the start; a chart asking for
    /// another adds it. An empty list turns the recording off.
    #[arg(long, env = "TRADOOR_OPEN_INTEREST", default_value = "BTC,ETH,SOL", value_delimiter = ',')]
    open_interest: Vec<String>,
    /// A Telegram bot's token (from @BotFather), for alerts sent to a chat.
    /// Without it the Telegram section of Settings says it is not set up.
    #[arg(long, env = "TRADOOR_TELEGRAM_TOKEN", default_value = "", hide_env_values = true)]
    telegram_token: String,
    /// The site's address (https://your-domain), for the link in a Telegram
    /// message that opens its chart. Without it the messages carry none.
    #[arg(long, env = "TRADOOR_SITE_URL", default_value = "")]
    site_url: String,
    /// Footprint history to read from Binance's archive of every trade, as
    /// market@first-day — binance-perp:BTCUSDT@2026-01-01 — a month at a time.
    /// Empty reads none: the history is then what the server recorded itself.
    #[arg(long, env = "TRADOOR_FOOTPRINT_HISTORY", default_value = "", value_delimiter = ',')]
    footprint_history: Vec<String>,
    /// A key of 32 characters or more opens the owner's status page
    /// (/online.html): readers on screen, Telegram accounts, the server's state.
    /// Without one the page has nothing to read.
    #[arg(long, env = "TRADOOR_ADMIN_KEY", default_value = "", hide_env_values = true)]
    admin_key: String,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server (the default).
    Serve,
    /// Import drawings exported from rsalianto (JSON lines; see scripts/export-rsalianto-drawings.sh).
    ImportDrawings {
        file: PathBuf,
        /// The browser id they go to (Settings → Browser id); without it they are nobody's until adopted.
        #[arg(long, default_value = "")]
        owner: String,
    },
    /// Give the drawings and workspace that have no owner — everything from
    /// before browsers had ids — to one browser id (Settings → Browser id).
    Adopt { owner: String },
    /// Replay the screener's four-hour rules over a long history of the coins
    /// it screens and write each pattern, with what was known at its entry and
    /// the bars after, as JSON lines (see research.rs). Needs no database.
    SignalResearch {
        out: PathBuf,
        #[arg(long, default_value_t = signals::COINS)]
        coins: usize,
        /// Four-hour bars read per coin: 3000 is some five hundred days.
        #[arg(long, default_value_t = 3000)]
        bars: usize,
    },
}

/// Connections for the requests readers make, and how long one of their queries may run.
const REQUEST_CONNECTIONS: usize = 24;
const REQUEST_STATEMENT_MS: u64 = 20_000;
/// Connections for the recorders and watchers working in the background.
const WORK_CONNECTIONS: usize = 12;

/// A pool of `size` connections to Postgres. Whoever asks for one waits ten
/// seconds at most and then fails, as a statement does past `statement_ms`: the
/// defaults were two connections a core and no limit on either, and with three
/// hundred readers at once every request waited behind the one before it until
/// none was answered at all.
fn pool(url: &str, size: usize, statement_ms: Option<u64>) -> anyhow::Result<deadpool_postgres::Pool> {
    let secs = |s| Some(std::time::Duration::from_secs(s));
    deadpool_postgres::Config {
        url: Some(url.to_string()),
        // A transaction left open cannot hold its connection either.
        options: statement_ms.map(|ms| format!("-c statement_timeout={ms} -c idle_in_transaction_session_timeout={}", 3 * ms)),
        pool: Some(deadpool_postgres::PoolConfig { max_size: size, timeouts: deadpool_postgres::Timeouts { wait: secs(10), create: secs(10), recycle: secs(5) }, ..Default::default() }),
        ..Default::default()
    }
    .create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls)
    .context("creating the connection pool")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let cli = Cli::parse();
    if let Some(Command::SignalResearch { out, coins, bars }) = &cli.command {
        return research::run(cli.binance_spot_rest.clone(), out, *coins, *bars).await;
    }
    let database_url = cli.database_url.clone().context("DATABASE_URL is required")?;

    // Migrations run on a dedicated connection before the pool opens, so every
    // pooled connection starts on the current schema.
    let mut db = Db::connect(&database_url).await.context("connecting to Postgres")?;
    db.migrate().await?;

    let pool = pool(&database_url, REQUEST_CONNECTIONS, Some(REQUEST_STATEMENT_MS))?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::ImportDrawings { file, owner } => {
            if !owner.is_empty() && !api::valid_owner(&owner) {
                anyhow::bail!("--owner must be a browser id: 16 to 64 letters, digits and dashes");
            }
            import::run(&pool, &file, &owner).await
        }
        Command::SignalResearch { .. } => unreachable!("run before the database"),
        Command::Adopt { owner } => {
            if !api::valid_owner(&owner) {
                anyhow::bail!("a browser id is 16 to 64 letters, digits and dashes — copy it from Settings");
            }
            let client = pool.get().await?;
            let drawings = client.execute("UPDATE drawing SET owner = $1 WHERE owner = ''", &[&owner]).await?;
            // The browser's own workspace, if it saved one, stays; the old shared one is dropped then.
            let workspaces = client.execute("UPDATE workspace w SET owner = $1 WHERE owner = '' AND NOT EXISTS (SELECT 1 FROM workspace o WHERE o.owner = $1 AND o.id = w.id)", &[&owner]).await?;
            tracing::info!(drawings, workspaces, owner = api::short_owner(&owner), "adopted");
            Ok(())
        }
        Command::Serve => {
            let tickers: Vec<String> = cli.liquidations.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
            if let Some(bad) = tickers.iter().find(|t| !liquidations::is_ticker(t)) {
                anyhow::bail!("TRADOOR_LIQUIDATIONS: {bad:?} is not a ticker — write them like BTCUSDT,ETHUSDT");
            }
            let doh = dns::Doh::new()?;
            let coins: Vec<String> = cli.open_interest.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
            if let Some(bad) = coins.iter().find(|c| open_interest::Coin::parse(c).is_none()) {
                anyhow::bail!("TRADOOR_OPEN_INTEREST: {bad:?} is not a coin — write them like BTC,ETH,1000PEPE");
            }
            // Background work has connections of its own, so a burst of readers and a
            // recorder's writes never wait on each other; its queries may run long.
            let work = self::pool(&database_url, WORK_CONNECTIONS, None)?;
            let open_interest = if coins.is_empty() { None } else { Some(open_interest::Recorder::start(work.clone(), coins, &doh).await?) };
            let token = cli.telegram_token.trim().to_string();
            let sessions = session::Sessions::load(pool.clone()).await?;
            let telegram = if token.is_empty() { None } else { Some(telegram::Telegram::start(pool.clone(), token, &doh, sessions.clone())?) };
            let catalog = Arc::new(catalog::Catalog::new(cli.binance_spot_rest.clone(), &doh)?);
            let indices = indices::Indices::start(pool.clone(), &doh)?;
            catalog.list_indices(indices.clone());
            catalog.keep_fresh();
            let econ = econ::Calendar::start(&doh)?;
            alert_watch::start(work.clone(), doh.clone(), catalog.clone(), telegram.clone());
            let flow = Some(flow::Flow::start(work.clone(), doh.clone(), catalog.clone(), telegram.clone()));
            footprint::Footprint::start(work.clone(), doh.clone(), catalog.clone());
            let history: Vec<String> = cli.footprint_history.iter().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()).collect();
            footprint_history::start(work.clone(), &doh, cli.data_dir.join("footprint-archive"), history)?;
            journal::start(work.clone(), &doh, catalog.clone());
            let site = Some(cli.site_url.trim().to_string()).filter(|s| !s.is_empty());
            signals::start(work.clone(), &doh, catalog.clone(), telegram.clone(), site);
            let training = training::Live::new();
            training::start(work.clone(), doh.clone(), training.clone());
            let state = api::AppState {
                econ,
                open_interest,
                flow,
                telegram,
                liquidations: (!tickers.is_empty()).then(|| liquidations::Collector::start(work.clone(), tickers, doh.clone())),
                pool,
                work,
                presence: presence::Presence::start(&cli.admin_key),
                data: DataDir::new(&cli.data_dir),
                catalog,
                relay: relay::Relay::new(doh)?,
                limits: Arc::new(limits::Limits::default()),
                indices,
                training,
                sessions,
            };
            let app = api::router(state, cli.data_dir.clone(), cli.web_dir.clone());
            let listener = tokio::net::TcpListener::bind(cli.bind).await.with_context(|| format!("binding {}", cli.bind))?;
            tracing::info!(addr = %cli.bind, "listening");
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
            Ok(())
        }
    }
}
