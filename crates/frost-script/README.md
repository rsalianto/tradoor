# Frost Script

tradoor's own language for indicators and strategies. A script reads the chart's candles and draws on the chart, or gives orders for a backtest; it has no way to reach the page, the network, storage or an account.

```
input length = 20 min 2 max 500 "Length"
average = ema(close, length)
plot(average, "Average", blue)
mark(crossover(close, average), "Up", below, green)
```

- **Reference:** https://tradoorterminal.com/doc.html
- **AI instructions** for turning a Pine Script v6 indicator into Frost Script: [`web/public/frost-script-ai.txt`](../../web/public/frost-script-ai.txt) (the editor copies them)
- **Community scripts, drawn live:** https://tradoorterminal.com/community.html

## What it does

- **Draws:** lines, histograms, levels, fills, marks, labels, boxes, a table, background shading and candle colours, on the price chart or in a pane of its own.
- **Backtests:** a script that gives orders (`long`, `short`, `exit`, `stop`, `target`) is a strategy. It is filled bar by bar as an exchange would fill it: at the next open, with fees, slippage, funding and liquidation, and with a lower timeframe's bars deciding a bar that reached both its stop and its target ([`src/strategy.rs`](src/strategy.rs)).
- **Language:** numbers, texts, lists, functions of your own, `if`, `for … by`, `while`, `break`, `continue`, `state` kept from bar to bar, `x[n]` to look back.
- **Built in:** the usual averages and windows (`sma`, `ema`, `rsi`, `atr`, `highest`, `pivothigh`, `crossover`…), maths, dates, and colours as `#hex`, `rgb()` and `alpha()`.
- **Settings:** numbers, tick boxes, colours, choices and dates, shown in the chart's settings like any built-in's.

## Safe by design

A script someone else wrote can draw a wrong line at worst.

- **Nothing to reach out with:** it runs in its own WebAssembly module that imports nothing, and the language has no word for the network, the page or an account.
- **Bounded work, memory and depth:** every limit is in [`src/lib.rs`](src/lib.rs). A script that passes one stops with an error naming the line.
  - **Length:** 40,000 characters, 12,000 words and signs.
  - **Nesting:** brackets and blocks 40 deep, one expression 100, and the whole script 200 once its functions are written out where they are called.
  - **Work:** 40 million steps a run, where long texts, sorts and windows cost what they do, so a script that asks too much stops within about a second.
  - **Memory:** a million past values across every window and `[n]`, lists of 5,000, and 500 lines, 500 labels and 200 boxes.
- **Checked on the server, isolated:** every saved script is checked with this crate on a thread of its own, four at a time, so no script can stop the server.
- **Text is text:** what a script writes reaches the page as plain text, never as markup.

## How it's built

```text
src/syntax.rs    text → tokens → tree; every node counts its depth as it is made
src/compile.rs   the tree checked and written out: names, kinds, functions inlined per call
src/run.rs       the per-bar machine, counting steps; lists are rings
src/lib.rs       the limits, the output types, `run` and `describe`
src/tests.rs     every built-in source against tradoor's own indicators, mistakes, limits, hostile scripts
sources/*.frost  tradoor's 18 built-in indicators written in Frost Script, and a SuperTrend example
```

`crates/frost-script-wasm` is the browser's module: a C ABI of bytes and numbers, no imports, loaded by the chart's worker only when a chart has a script. The server (`bins/server/src/scripts.rs`) checks saved scripts with the same crate.

Each built-in's file gives exactly the numbers of the indicator it is named after, and shows in that indicator's Source tab. They are the best place to start.

## Tests

```sh
cargo test -p tradoor-frost-script
```

## License

Part of tradoor, under the [GNU Affero General Public License, version 3](../../LICENSE).
