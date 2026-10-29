/**
 * The TPO settings dialog's layout: MMT's tabs, field names and hints.
 * Loaded with the dialog; the defaults themselves live in tpo-settings, which
 * the chart needs from the start.
 */
import { TPO_DEFAULTS } from './tpo-settings';

interface Base {
  key: string;
  label: string;
  hint?: string;
}

export type TpoField =
  | (Base & { kind: 'bool'; default: boolean })
  | (Base & { kind: 'number'; default: number; min: number; max: number; step?: number })
  | (Base & { kind: 'color'; default: string })
  | (Base & { kind: 'choice'; default: string; options: readonly { value: string; label: string }[] })
  /** Ticks per row, with an auto box: 0 is auto. */
  | (Base & { kind: 'ticks'; default: number })
  /** The custom sessions' list. */
  | (Base & { kind: 'sessions'; default: string })
  /** How long a profile runs: the chart's own TPO style, not a setting of its own. */
  | (Base & { kind: 'period'; default: string });

export interface TpoSection {
  title: string;
  fields: readonly TpoField[];
}

// Defaults are read from TPO_DEFAULTS, so the dialog cannot disagree with the chart.
const bool = (key: string, label: string, hint?: string): TpoField => ({ key, label, kind: 'bool', default: TPO_DEFAULTS[key] === '1', hint });
const color = (key: string, label: string, hint?: string): TpoField => ({ key, label, kind: 'color', default: TPO_DEFAULTS[key] ?? '#000000', hint });
const number = (key: string, label: string, min: number, max: number, step?: number, hint?: string): TpoField => ({ key, label, kind: 'number', default: Number(TPO_DEFAULTS[key]), min, max, step, hint });
const choice = (key: string, label: string, options: readonly { value: string; label: string }[], hint?: string): TpoField => ({ key, label, kind: 'choice', default: TPO_DEFAULTS[key] ?? '', options, hint });
const naked = (key: string): TpoField => bool(key, 'Naked Extension', 'Runs right until price trades back to it, not just to the next profile');

export const TPO_SECTIONS: readonly TpoSection[] = [
  {
    title: 'General',
    fields: [
      { key: 'rowTicks', label: 'Ticks per row', kind: 'ticks', default: 0, hint: 'How much price a row holds. Auto cuts a median day into about fifty rows; MMT uses 500–750 on BTC' },
      color('profileColor', 'Profile Color', 'Blocks outside the value area'),
      choice(
        'blockMode',
        'Block Color Mode',
        [
          { value: 'brightness', label: 'Brightness' },
          { value: 'solid', label: 'Solid' },
          { value: 'heatmap', label: 'Heat map' },
          { value: 'candle', label: 'Candle up/down' },
        ],
        'Brightness: a block lighter the later its bracket printed',
      ),
      color('heatLow', 'Heat map, fewest'),
      color('heatHigh', 'Heat map, most'),
      bool('highlight', 'Highlight start end', 'The open’s block and the close’s block in their own colours'),
      color('startColor', 'Start block color'),
      color('endColor', 'End block color'),
      bool('hideText', 'Hide Text', 'No POC, VAH, VAL, PH or PL names'),
      number('spacing', 'Profile spacing', 0, 50, 1, 'Columns between neighbouring profiles'),
    ],
  },
  {
    title: 'Session',
    fields: [
      { key: 'period', label: 'Session Period', kind: 'period', default: '1d', hint: 'One profile a day, week or month, or a session of each day. Sessions need bars of an hour or less' },
      { key: 'sessions', label: 'Custom Sessions', kind: 'sessions', default: TPO_DEFAULTS.sessions ?? '', hint: 'Used by the Custom period, in UTC. An end at or before its start runs past midnight' },
      bool('sessionHeader', 'Session Header', 'Each profile’s session or day written over it'),
    ],
  },
  {
    title: 'Point of Control',
    fields: [bool('poc', 'Show POC'), number('pocWidth', 'Thickness', 1, 6), color('pocColor', 'Profile Color', 'The POC row’s blocks'), color('pocRayColor', 'Ray Color'), naked('nakedPoc')],
  },
  {
    title: 'Value Area',
    fields: [
      number('vaPct', 'Value Area %', 1, 100, 1, 'Share of the brackets the value area holds, grown out from the POC'),
      color('vaColor', 'Profile Color', 'Blocks inside the value area'),
      bool('vaRays', 'Show Value Area Rays'),
      color('vaRayColor', 'Ray Color'),
      naked('nakedVa'),
      bool('fillVa', 'Fill Value Area'),
      color('fillVaColor', 'Fill Color'),
    ],
  },
  {
    title: 'Single Prints',
    fields: [bool('singlePrints', 'Show Single Prints', 'Rows only one bracket traded, inside the profile: gaps price tends to come back and fill'), color('singlePrintColor', 'Profile Color'), color('singlePrintRayColor', 'Ray Color'), naked('nakedSinglePrints')],
  },
  {
    title: 'Poor High Low',
    fields: [bool('poor', 'Show Poor High Low', 'A high or low two brackets or more share, with no tail: unfinished business'), color('poorColor', 'Poor High Low Color'), naked('nakedPoor')],
  },
  {
    title: 'Initial Balance',
    fields: [bool('ib', 'Show Initial Balance'), color('ibColor', 'Color'), number('ibCandles', 'Candle Count', 1, 100, 1, 'Two 30-minute candles: the first hour')],
  },
  {
    title: 'Volume Profile',
    fields: [
      bool('vp', 'Show Volume Profile', 'Volume against time, beside each profile'),
      color('vpBuy', 'Bought'),
      color('vpSell', 'Sold'),
      choice(
        'vpMode',
        'Render Mode',
        [
          { value: 'standard', label: 'Standard' },
          { value: 'total', label: 'Total Volume' },
          { value: 'delta', label: 'Delta' },
        ],
        'Bought and sold need the market’s trades, which tradoor records for a few markets; elsewhere it is total volume',
      ),
      color('vpColor', 'Total Volume color'),
      choice('vpOrigin', 'Origin', [
        { value: 'left', label: 'left' },
        { value: 'right', label: 'right' },
      ]),
      number('vpWidth', 'Volume Profile Width', 0.1, 1, 0.1, 'Its longest row, as a share of the profile’s widest'),
      bool('vpVaRays', 'Show Value Area Rays'),
      color('vpVaColor', 'Value area color'),
      naked('vpNakedVa'),
      number('vpVaPct', 'Value Area %', 1, 100),
      bool('vpLabels', 'Volume Labels', 'Each row’s volume, where rows are tall enough to read'),
      color('vpLabelColor', 'Label color'),
      bool('vpPoc', 'Show POC'),
      color('vpPocColor', 'POC color'),
      naked('vpNakedPoc'),
      bool('vpHideText', 'Hide Text', 'No labels on the volume profile'),
      number('vpBinFilter', 'Bin filter', 0, 100, 1, 'Rows under this share of the busiest row, in percent, are left out'),
    ],
  },
];
