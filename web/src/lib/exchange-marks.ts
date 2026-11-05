/**
 * The live exchanges' marks as plain path data, so both the DOM logos and the
 * chart engine draw the same artwork. The engine's text layer fills these with
 * Path2D on its 2D canvas; nothing here imports Solid, so the worker can use it.
 */
import type { ExchangeId } from './exchanges';

/** One filled path, in the mark's own square box. */
export interface MarkPath {
  d: string;
  fill: string;
  /** The group's `translate(tx ty) scale(s)`, applied in that order. */
  t?: [tx: number, ty: number, s: number];
}

export interface ExchangeMark {
  /** Side of the square the paths are drawn in. */
  box: number;
  paths: MarkPath[];
}

/** The black disc every mark sits on, as a path rather than a <circle>. */
const DISC = { d: 'M0 16a16 16 0 1 0 32 0a16 16 0 1 0-32 0', fill: '#000' };
/** The same disc for a mark drawn in a 16-unit box. */
const DISC16 = { d: 'M0 8a8 8 0 1 0 16 0a8 8 0 1 0-16 0', fill: '#000' };

export const MARKS: Record<ExchangeId, ExchangeMark> = {
  binance: {
    box: 32,
    paths: [
      DISC,
      {
        t: [-5, -5, 1.75],
        fill: '#F0B90B',
        d: 'M7.6159 12L5.81135 13.8045L4.00226 12L5.80681 10.1955L7.6159 12ZM12.0023 7.61364L15.0977 10.7091L16.9023 8.90455L12.0023 4L7.09772 8.90455L8.90226 10.7091L12.0023 7.61364ZM18.1932 10.1955L16.3886 12L18.1932 13.8045L19.9977 12L18.1932 10.1955ZM12.0023 16.3864L8.90681 13.2909L7.10226 15.0955L12.0023 20L16.9023 15.0955L15.0977 13.2909L12.0023 16.3864ZM12.0023 13.8045L13.8068 12L12.0023 10.1955L10.1932 12L12.0023 13.8045Z',
      },
    ],
  },
  bybit: {
    box: 32,
    paths: [
      DISC,
      { t: [-5, -5, 1.75], fill: '#F6A500', d: 'M15.4033 13.445V9.33372H16.23V13.445H15.4033Z' },
      {
        t: [-5, -5, 1.75],
        fill: '#FFF',
        d: 'M5.77161 14.6663H4V10.555H5.70075C6.5275 10.555 7.00819 11.0049 7.00819 11.71C7.00819 12.1659 6.69875 12.4612 6.48498 12.5592C6.74009 12.675 7.06725 12.9348 7.06725 13.4828C7.06725 14.2505 6.52632 14.6663 5.77161 14.6663ZM5.63461 11.2707H4.82557V12.2179H5.63461C5.98538 12.2179 6.18144 12.0278 6.18144 11.7443C6.18144 11.462 5.98538 11.2707 5.63461 11.2707ZM5.68775 12.9395H4.82557V13.9505H5.68775C6.06215 13.9505 6.2405 13.7202 6.2405 13.4427C6.2405 13.1651 6.06215 12.9407 5.68775 12.9407V12.9395Z',
      },
      { t: [-5, -5, 1.75], fill: '#FFF', d: 'M9.58884 12.9797V14.6663H8.76799V12.9797L7.49598 10.555H8.39359L9.18373 12.212L9.96324 10.555H10.8609L9.58884 12.9797Z' },
      {
        t: [-5, -5, 1.75],
        fill: '#FFF',
        d: 'M13.2041 14.6663H11.4325V10.555H13.1332C13.96 10.555 14.4407 11.0049 14.4407 11.71C14.4407 12.1659 14.1312 12.4612 13.9175 12.5592C14.1726 12.675 14.4997 12.9348 14.4997 13.4828C14.4997 14.2505 13.9588 14.6663 13.2041 14.6663ZM13.0671 11.2707H12.2581V12.2179H13.0671C13.4179 12.2179 13.6139 12.0278 13.6139 11.7443C13.6139 11.462 13.4179 11.2707 13.0671 11.2707ZM13.1202 12.9395H12.2581V13.9505H13.1202C13.4946 13.9505 13.673 13.7202 13.673 13.4427C13.673 13.1651 13.4946 12.9407 13.1202 12.9407V12.9395Z',
      },
      { t: [-5, -5, 1.75], fill: '#FFF', d: 'M18.8933 11.2707V14.6663H18.0678V11.2707H16.9611V10.555H20V11.2707H18.8933Z' },
    ],
  },
  okx: {
    box: 32,
    paths: [DISC, { t: [2, 2, 1.16667], fill: '#FFF', d: 'M3 3h6v6H3zm12 6H9v6H3v6h6v-6h6v6h6v-6h-6zm0 0V3h6v6z' }],
  },
  bitget: {
    box: 32,
    paths: [
      DISC,
      { t: [2, 2, 1.16667], fill: '#00F0FF', d: 'M11.121 9.46h4.283l4.381 4.555a.785.785 0 0 1 .003 1.076L14.17 21H9.757l1.334-1.357l4.898-5.092l-4.836-5.092' },
      { t: [2, 2, 1.16667], fill: '#00F0FF', d: 'M12.879 14.54H8.596L4.215 9.986a.785.785 0 0 1-.003-1.076L9.83 3h4.412l-1.334 1.357L8.01 9.449l4.836 5.092' },
    ],
  },
  gate: {
    box: 32,
    paths: [
      DISC,
      { t: [5, 5, 0.036667], fill: '#2354E6', d: 'M300 465a165 165 0 0 1 0-330V0a300 300 0 1 0 300 300H465a165 165 0 0 1-165 165' },
      { t: [5, 5, 0.036667], fill: '#17E6A1', d: 'M300 300h165V135H300z' },
    ],
  },
  htx: {
    box: 16,
    paths: [
      DISC16,
      { t: [1.2, 1.2, 0.85], fill: '#1b2143', d: 'M9.8 4.92A6.73 6.73 0 0 0 7.83 0h-.06C7.7 4.5 5.43 5.7 4.2 7.32a5.43 5.43 0 0 0 2.52 8.59c1.52.42-.35-.75-.6-3.23-.3-3 3.7-5.28 3.7-7.76z' },
      { t: [1.2, 1.2, 0.85], fill: '#2ca6e0', d: 'M11.1 6.43h-.06A8.17 8.17 0 0 1 9.7 9.29c-2.45 3.72-1 5.5-.26 6.48.45.56 0 0 1.13-.57a5.16 5.16 0 0 0 2.46-4 5.77 5.77 0 0 0-1.93-4.8z' },
    ],
  },
  bitfinex: {
    box: 16,
    paths: [
      DISC16,
      { t: [1.2, 1.2, 0.85], fill: '#97c554', d: 'M.6 13.73c.73.8 4.8 4.57 11.15.2 4.6-3.62 4.48-11.46 4-13.83C15.64.43 10.3 12.25.6 13.73z' },
      { t: [1.2, 1.2, 0.85], fill: '#719b37', d: 'M15.8.1A18.59 18.59 0 0 0 4 3.06a8.49 8.49 0 0 0-4 8.12C8.53 10.22 15.6.36 15.8.1z' },
    ],
  },
  aster: {
    box: 32,
    paths: [
      DISC,
      // The official mark is a gradient; the engine fills flat, so its middle colour.
      { t: [6.1, 6.1, 0.62], fill: '#F9D3A8', d: 'm9.13 30.44.75-3.45a6.19 6.19 0 0 0-6.06-7.5H.4a16.04 16.04 0 0 0 8.74 10.95Z' },
      { t: [6.1, 6.1, 0.62], fill: '#F9D3A8', d: 'M10.64 31.07A16.08 16.08 0 0 0 31.73 19.5H21.25c-4.73 0-8.81 3.28-9.81 7.88l-.8 3.69Z' },
      { t: [6.1, 6.1, 0.62], fill: '#F9D3A8', d: 'M32 17.9A16.03 16.03 0 0 0 17.37.05l-2.24 10.36a6.19 6.19 0 0 0 6.06 7.49H32Z' },
      { t: [6.1, 6.1, 0.62], fill: '#F9D3A8', d: 'M15.75 0A16.03 16.03 0 0 0 .1 17.9h3.65c4.73 0 8.81-3.28 9.81-7.89L15.75 0Z' },
    ],
  },
  // The series the tradoor server works out: three rising bars.
  index: {
    box: 32,
    paths: [DISC, { fill: '#FFF', d: 'M8.5 19h3.5v5.5H8.5zM14.25 13.5h3.5v11h-3.5zM20 8h3.5v16.5H20z' }],
  },
  lighter: {
    box: 32,
    paths: [DISC, { t: [10.6, 5.9, 0.78], fill: '#FFF', d: 'M5.989 20.388 0 25.498V5.888L5.989.502v19.886Zm7.811.012-5.989 5.098V17.85l5.989-5.37v7.92Z' }],
  },
  kraken: {
    box: 32,
    paths: [
      { d: DISC.d, fill: '#7132F5' },
      // A dome on four legs.
      {
        fill: '#FFF',
        d: 'M7 22.4V17A9 9 0 0 1 25 17V22.4A1.6 1.6 0 0 1 21.8 22.4V18A.865.865 0 0 0 20.07 18V22.4A1.6 1.6 0 0 1 16.87 22.4V18A.865.865 0 0 0 15.13 18V22.4A1.6 1.6 0 0 1 11.93 22.4V18A.865.865 0 0 0 10.2 18V22.4A1.6 1.6 0 0 1 7 22.4Z',
      },
    ],
  },
  deribit: {
    box: 32,
    paths: [DISC, { fill: '#00CFBE', d: 'M10 9H16.5A7 7 0 0 1 16.5 23H10ZM13 12V20H16.5A4 4 0 0 0 16.5 12Z' }],
  },
  hyperliquid: {
    box: 32,
    paths: [DISC, { fill: '#97FCE4', d: 'M8 16c0-4 2.5-6 5-6 3 0 3.5 3 5.5 3S21 10 23.5 10 24 13 24 16s-.5 6-3 6-3-3-5-3-2.5 3-5.5 3S8 20 8 16z' }],
  },
};

export { MARK_ORDER, markIndex } from './mark-order';

/**
 * The mark as the engine takes it: one path a line, `tx|ty|scale|fill|d`.
 * A plain string keeps the worker boundary free of JSON and the engine free
 * of a parser it would only use here.
 */
export const markSpec = (mark: ExchangeMark): string =>
  mark.paths.map((p) => `${p.t?.[0] ?? 0}|${p.t?.[1] ?? 0}|${p.t?.[2] ?? 1}|${p.fill}|${p.d}`).join('\n');
