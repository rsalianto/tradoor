/**
 * The built-in indicators written as formulas (crates/formula/sources, which
 * the formula crate's tests hold to the indicators' own numbers), and the new
 * formula's first text. Loaded with the editor and the settings dialog alone.
 */
import atr from '../../../crates/formula/sources/atr.frost?raw';
import bb from '../../../crates/formula/sources/bb.frost?raw';
import ema from '../../../crates/formula/sources/ema.frost?raw';
import keltner from '../../../crates/formula/sources/keltner.frost?raw';
import macd from '../../../crates/formula/sources/macd.frost?raw';
import rsi from '../../../crates/formula/sources/rsi.frost?raw';
import sma from '../../../crates/formula/sources/sma.frost?raw';
import stochrsi from '../../../crates/formula/sources/stochrsi.frost?raw';
import supertrend from '../../../crates/formula/sources/supertrend.frost?raw';
import vwap from '../../../crates/formula/sources/vwap.frost?raw';

/** The built-in indicators' workings, by study kind, as formulas: what their Source tab shows. */
export const SOURCES: Record<string, { name: string; source: string }> = {
  sma: { name: 'Simple Moving Average', source: sma },
  ema: { name: 'Exponential Moving Average', source: ema },
  bb: { name: 'Bollinger Bands', source: bb },
  keltner: { name: 'Keltner Channels', source: keltner },
  rsi: { name: 'Relative Strength Index', source: rsi },
  stochrsi: { name: 'Stochastic RSI', source: stochrsi },
  macd: { name: 'MACD', source: macd },
  atr: { name: 'Average True Range', source: atr },
  vwap: { name: 'VWAP', source: vwap },
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

