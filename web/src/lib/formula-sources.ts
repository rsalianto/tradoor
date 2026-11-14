/**
 * The built-in indicators written as formulas (crates/frost-script/sources, which
 * the formula crate's tests hold to the indicators' own numbers), and the new
 * formula's first text. Loaded with the editor and the settings dialog alone.
 */
import atr from '../../../crates/frost-script/sources/atr.frost?raw';
import avwap from '../../../crates/frost-script/sources/avwap.frost?raw';
import bb from '../../../crates/frost-script/sources/bb.frost?raw';
import ema from '../../../crates/frost-script/sources/ema.frost?raw';
import ema200 from '../../../crates/frost-script/sources/ema200.frost?raw';
import ichimoku from '../../../crates/frost-script/sources/ichimoku.frost?raw';
import keltner from '../../../crates/frost-script/sources/keltner.frost?raw';
import keylevels from '../../../crates/frost-script/sources/keylevels.frost?raw';
import macd from '../../../crates/frost-script/sources/macd.frost?raw';
import moon from '../../../crates/frost-script/sources/moon.frost?raw';
import ribbon from '../../../crates/frost-script/sources/ribbon.frost?raw';
import rsi from '../../../crates/frost-script/sources/rsi.frost?raw';
import sessions from '../../../crates/frost-script/sources/sessions.frost?raw';
import sma from '../../../crates/frost-script/sources/sma.frost?raw';
import stochrsi from '../../../crates/frost-script/sources/stochrsi.frost?raw';
import supertrend from '../../../crates/frost-script/sources/supertrend.frost?raw';
import swingmap from '../../../crates/frost-script/sources/swingmap.frost?raw';
import volume from '../../../crates/frost-script/sources/volume.frost?raw';
import vwap from '../../../crates/frost-script/sources/vwap.frost?raw';

/** The built-in indicators' workings, by study kind or layer, as scripts: what their Source tab shows. */
export const SOURCES: Record<string, { name: string; source: string }> = {
  ribbon: { name: 'EMA 12/25 Ribbon', source: ribbon },
  ema200: { name: 'EMA 200', source: ema200 },
  volume: { name: 'Volume', source: volume },
  sma: { name: 'Simple Moving Average', source: sma },
  ema: { name: 'Exponential Moving Average', source: ema },
  bb: { name: 'Bollinger Bands', source: bb },
  keltner: { name: 'Keltner Channels', source: keltner },
  ichimoku: { name: 'Ichimoku Cloud', source: ichimoku },
  rsi: { name: 'Relative Strength Index', source: rsi },
  stochrsi: { name: 'Stochastic RSI', source: stochrsi },
  macd: { name: 'MACD', source: macd },
  atr: { name: 'Average True Range', source: atr },
  vwap: { name: 'VWAP', source: vwap },
  avwap: { name: 'Anchored VWAP', source: avwap },
  keylevels: { name: 'Historical Key Levels', source: keylevels },
  swingmap: { name: 'Swing Structure Map', source: swingmap },
  sessions: { name: 'Trading Sessions', source: sessions },
  moon: { name: 'Moon Phases', source: moon },
};

/** The built-ins a script cannot be written for, and why: each reads what a script is not given. */
export const UNWRITTEN: Record<string, string> = {
  liquidations: 'It reads the liquidations the exchanges report as they happen. A script is given the chart’s candles alone.',
  volumedelta: 'It reads what market buyers took in each bar, which the exchange gives the chart beside its candles. A script is given the candles alone.',
  vpvr: 'It reads the bars on screen, which change as you scroll and zoom. A script works out each bar from the bars before it.',
  vwaplevels: 'It reads hourly and daily bars the chart loads beside its own, back to the start of last year, so a minute chart has the year’s levels. A script is given the chart’s own bars alone.',
  oi: 'It reads open interest from the exchanges, beside the candles. A script is given the candles alone.',
  flow: 'It reads the trades of several exchanges as they happen. A script is given the chart’s candles alone.',
};

/** To start from in the editor: the built-ins, and one that is not. */
export const EXAMPLES: { name: string; source: string }[] = [{ name: 'SuperTrend', source: supertrend }, ...Object.values(SOURCES)];

export const NEW_FORMULA = `// My indicator
// Lines that start with // are notes. Each line below is worked out on every bar.
input length = 20 min 2 max 500 "Length"
average = ema(close, length)
plot(average, "Average", blue)
mark(crossover(close, average), "Up", below, green)
`;

