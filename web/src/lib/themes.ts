/**
 * Themes: every colour a workspace can override (lib/theme), picked whole in
 * Settings. tradoor dark is the defaults themselves; the rest follow themes
 * TradingView traders share (tradingviewthemes.com), whose chart colours they
 * keep — background, grid, axis text, crosshair, candles — with a page around
 * the chart to match, light or dark.
 */
import { CHART_THEME, SCHEME, UI_THEME } from './theme';

export interface ThemePreset {
  id: string;
  name: string;
  /** What it is, for the card. */
  note: string;
  colors: Record<string, string>;
}

/** Every key a theme sets: picking one clears what another set and it does not. */
export const THEME_KEYS: readonly string[] = [...CHART_THEME.map((t) => t.name), ...UI_THEME.map((t) => t.name), SCHEME.name];

/** TradingView's own study colours, for the themes that come from it. */
const TV_STUDIES = { study0: '#2962FF', study1: '#9C27B0', study2: '#FF9800', study3: '#00BCD4', study4: '#E91E63', study5: '#4CAF50' };
const PASTEL_STUDIES = { study0: '#5B7DB1', study1: '#A78BCB', study2: '#E0A458', study3: '#6CB4A8', study4: '#D98BA3', study5: '#8FBF7A' };

export const THEMES: readonly ThemePreset[] = [
  { id: 'tradoor-dark', name: 'tradoor dark', note: 'The terminal as it comes: lime on near-black', colors: {} },
  {
    id: 'canvas',
    name: 'Canvas',
    note: "TradingView's own dark: navy, teal and red",
    colors: {
      background: '#131722', border: '#20242F', text: '#B2B5BE', crosshair: '#9598A1', labelBackground: '#2A2E39',
      bull: '#089981', bear: '#F23645', ribbonFast: '#089981', ribbonSlow: '#F23645', ema200: '#F7A21B',
      drawing: '#2962FF', drawingSelected: '#7FA1FF', ...TV_STUDIES,
      'ui.primary': '#2962FF', 'ui.bg': '#0F121A', 'ui.surface': '#131722', 'ui.border': '#2A2E39', 'ui.text': '#D1D4DC', 'ui.bull': '#089981', 'ui.bear': '#F23645',
    },
  },
  {
    id: 'crayons-dark',
    name: 'Crayons Dark',
    note: 'Near-black, white and blue-grey candles',
    colors: {
      background: '#0F0F0F', border: '#1E1E1E', text: '#B2B5BE', crosshair: '#9698A1', labelBackground: '#242424',
      bull: '#F7F7F7', bear: '#8396B1', ribbonFast: '#F7F7F7', ribbonSlow: '#8396B1', ema200: '#E0A458',
      drawing: '#A9BCD9', drawingSelected: '#D6E0EE', ...PASTEL_STUDIES,
      'ui.primary': '#F7F7F7', 'ui.bg': '#080808', 'ui.surface': '#0F0F0F', 'ui.border': '#242424', 'ui.text': '#E6E6E6', 'ui.bull': '#F7F7F7', 'ui.bear': '#8396B1',
    },
  },
  {
    id: 'market-canvas',
    name: 'Market Canvas',
    note: 'Classic white, green and red candles',
    colors: {
      background: '#FFFFFF', border: '#E1E1E1', text: '#8B8BA7', crosshair: '#8B8BA7', labelBackground: '#E6E8EE',
      bull: '#079981', bear: '#F23645', ribbonFast: '#079981', ribbonSlow: '#F23645', ema200: '#F7A21B',
      drawing: '#2962FF', drawingSelected: '#7FA1FF', ...TV_STUDIES,
      'ui.primary': '#079981', 'ui.bg': '#F1F2F5', 'ui.surface': '#FFFFFF', 'ui.border': '#E1E1E1', 'ui.text': '#131722', 'ui.bull': '#079981', 'ui.bear': '#F23645',
      'ui.scheme': 'light',
    },
  },
  {
    id: 'crayons-light',
    name: 'Crayons Light',
    note: 'Soft grey, black and blue-grey candles',
    colors: {
      background: '#F7F7F7', border: '#ECEEF2', text: '#9EA2AB', crosshair: '#9698A1', labelBackground: '#E6E8EC',
      bull: '#0F0F0F', bear: '#8396B1', ribbonFast: '#3A3A3A', ribbonSlow: '#8396B1', ema200: '#E0A458',
      drawing: '#5B7DB1', drawingSelected: '#93A9CC', ...PASTEL_STUDIES,
      'ui.primary': '#0F0F0F', 'ui.bg': '#EFEFEF', 'ui.surface': '#F7F7F7', 'ui.border': '#E2E4E8', 'ui.text': '#0F0F0F', 'ui.bull': '#0F0F0F', 'ui.bear': '#8396B1',
      'ui.scheme': 'light',
    },
  },
  {
    id: 'ict-lime-green-black',
    name: 'ICT Lime Green Black',
    note: 'White, lime up and black down candles',
    colors: {
      background: '#FFFFFF', border: '#E0FBF7', text: '#40514E', crosshair: '#9598A1', labelBackground: '#E8F6F3',
      bull: '#2FB417', bear: '#000000', ribbonFast: '#2FB417', ribbonSlow: '#000000', ema200: '#30C5B1',
      drawing: '#14A38F', drawingSelected: '#7FD9CB', ...TV_STUDIES,
      'ui.primary': '#2FB417', 'ui.bg': '#F4F7F6', 'ui.surface': '#FFFFFF', 'ui.border': '#D9E8E5', 'ui.text': '#1C2826', 'ui.bull': '#2FB417', 'ui.bear': '#E53935',
      'ui.scheme': 'light',
    },
  },
];

/** The theme `theme` is exactly, or null once a colour of it was changed by hand. */
export function themeOf(theme: Record<string, string>): string | null {
  const same = (t: ThemePreset) => THEME_KEYS.every((k) => (theme[k] ?? '').toLowerCase() === (t.colors[k] ?? '').toLowerCase());
  return THEMES.find(same)?.id ?? null;
}

/** A theme's colours over `theme`, the keys it does not set put back to the defaults; the rest (font, time zone) kept. */
export function withTheme(theme: Record<string, string>, preset: ThemePreset): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(theme)) if (!THEME_KEYS.includes(k)) out[k] = v;
  return { ...out, ...preset.colors };
}
