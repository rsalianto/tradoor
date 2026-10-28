/** Colours a workspace can override. Chart keys go to the engine; `ui.*` keys
 *  set the page's CSS tokens. Defaults mirror tradoor-render's Theme and styles.css. */

export interface ThemeKey {
  name: string;
  label: string;
  value: string;
}

export const CHART_THEME: readonly ThemeKey[] = [
  { name: 'background', label: 'Background', value: '#0f0f0f' },
  { name: 'text', label: 'Axis text', value: '#7c847d' },
  { name: 'border', label: 'Grid and borders', value: '#232823' },
  { name: 'bull', label: 'Up candles', value: '#a4c588' },
  { name: 'bear', label: 'Down candles', value: '#d97161' },
  { name: 'ribbonFast', label: 'Ribbon, bullish', value: '#aafd35' },
  { name: 'ribbonSlow', label: 'Ribbon, bearish', value: '#ff293a' },
  { name: 'ema200', label: 'EMA 200', value: '#ffb03a' },
  { name: 'crosshair', label: 'Crosshair', value: '#7c847d' },
  { name: 'labelBackground', label: 'Axis labels', value: '#2a2f2a' },
  { name: 'drawing', label: 'Drawings', value: '#ff5fd2' },
  { name: 'drawingSelected', label: 'Selected drawing', value: '#ffa8e6' },
  { name: 'study0', label: 'Study line 1', value: '#7fb2ff' },
  { name: 'study1', label: 'Study line 2', value: '#c58fff' },
  { name: 'study2', label: 'Study line 3', value: '#ffd166' },
  { name: 'study3', label: 'Study line 4', value: '#4ecdc4' },
  { name: 'study4', label: 'Study line 5', value: '#ff8fab' },
  { name: 'study5', label: 'Study line 6', value: '#9ae66e' },
];

export const FONT_PX = { name: 'fontPx', value: '11' };

/** Whether the page is light or dark, for the browser's own controls and scrollbars: a theme's, not a swatch. */
export const SCHEME = { name: 'ui.scheme', css: '--scheme' };

export const UI_THEME: readonly (ThemeKey & { css: string })[] = [
  { name: 'ui.primary', css: '--primary', label: 'Accent', value: '#aafd35' },
  { name: 'ui.bg', css: '--bg', label: 'Page', value: '#070907' },
  { name: 'ui.surface', css: '--surface', label: 'Panels', value: '#0d100e' },
  { name: 'ui.border', css: '--border', label: 'Borders', value: '#232823' },
  { name: 'ui.text', css: '--text', label: 'Text', value: '#dfe6e0' },
  { name: 'ui.bull', css: '--bull', label: 'Up', value: '#aafd35' },
  { name: 'ui.bear', css: '--bear', label: 'Down', value: '#ff4d5f' },
];
