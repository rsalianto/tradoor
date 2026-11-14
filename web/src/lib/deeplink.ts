/**
 * A link that opens a chart, as Telegram's messages carry them: `?signal=<id>`
 * opens that pattern drawn, at its own length; `?market=…&tf=…` a market, and
 * with `&touched=<ms>` the touch of a line there, drawn. `?script=<id>`, from
 * the Community's showcase, puts that public Frost Script on the chart, and
 * `?indicator=<kind>` one of tradoor's own, as the showcase lists them too. Read
 * once the layout is in, then taken off the address so a reload does not open
 * it again.
 */
import { batch } from 'solid-js';
import { api, errorText } from './api';
import { openPattern, signalPattern, touchPattern } from './patterns';
import { isChartMarket } from './markets';
import { defaultId } from './study-schema';
import { splitList, withLayer } from './study-style';
import { studyCatalog } from '../engine/client';
import { toast } from './toasts';
import { patchPanel, setPanelMarket, setPanelTimeframe, targetChart } from '../workspace/store';
import type { ChartPanelState } from '../workspace/model';

/** What the engine keeps of a chart's list (`parse_studies`). */
const MOST_STUDIES = 12;

/** A public script from the showcase, on the chart: said whose it is, or why it is not there. */
async function addScript(id: string, target: ChartPanelState): Promise<void> {
  try {
    const s = await api.communityScript(id);
    const kind = `fx-${s.id}`;
    const studies = splitList(target.studies);
    if (!studies.some((x) => x.split(':')[0] === kind)) {
      if (studies.length >= MOST_STUDIES) {
        toast({ tone: 'warn', title: `A chart holds ${MOST_STUDIES} indicators`, body: `Remove one from the chart's legend, then add ${s.name} from the picker's Community tab.` });
        return;
      }
      patchPanel(target.id, { studies: [...studies, kind].join(',') });
    }
    toast({ tone: 'info', title: `${s.name}, by @${s.username}`, body: 'On the chart, from the Community. Its source is in its settings: read it before you trade on it.' });
  } catch (e) {
    toast({ tone: 'warn', title: 'Not added', body: (e as { status?: number }).status === 404 ? 'That script is no longer public.' : errorText(e) });
  }
}

/** The layers drawn on every chart, shown again rather than added. */
const LAYERS: Record<string, string> = { ribbon: 'EMA 12/25 ribbon', ema200: 'EMA 200', volume: 'Volume' };

/** One of tradoor's own indicators, from the showcase, on the chart with its usual settings. */
async function addIndicator(kind: string, target: ChartPanelState): Promise<void> {
  const layer = LAYERS[kind];
  if (layer) {
    patchPanel(target.id, withLayer(target, kind));
    toast({ tone: 'info', title: layer, body: 'On the chart.' });
    return;
  }
  const def = (await studyCatalog().catch(() => [])).find((d) => d.kind === kind);
  if (!def) return;
  const studies = splitList(target.studies);
  if (!studies.some((x) => x.split(':')[0] === kind)) {
    if (studies.length >= MOST_STUDIES) {
      toast({ tone: 'warn', title: `A chart holds ${MOST_STUDIES} indicators`, body: 'Remove one from the chart’s legend, then add it from the picker.' });
      return;
    }
    patchPanel(target.id, { studies: [...studies, defaultId(def)].join(',') });
  }
  toast({ tone: 'info', title: def.short, body: 'On the chart. Its settings, and its Frost Script source, are a double click on its name away.' });
}

export async function followLink(): Promise<void> {
  const url = new URL(location.href);
  const id = Number(url.searchParams.get('signal'));
  const market = url.searchParams.get('market');
  const tf = url.searchParams.get('tf');
  const touched = Number(url.searchParams.get('touched'));
  const script = url.searchParams.get('script');
  const indicator = url.searchParams.get('indicator');
  for (const key of ['signal', 'market', 'tf', 'touched', 'script', 'indicator']) url.searchParams.delete(key);
  history.replaceState(history.state, '', url.pathname + url.search + url.hash);
  const target = targetChart();
  if (!target) return;
  if (script && /^[a-z0-9]{6,16}$/.test(script)) await addScript(script, target);
  if (indicator && /^[a-z0-9]{2,16}$/.test(indicator)) await addIndicator(indicator, target);
  if (id || touched) {
    try {
      const state = await api.signals();
      const pattern = id ? [...state.signals, ...state.ended].find((s) => s.id === id) : undefined;
      if (pattern) return openPattern(signalPattern(pattern), target, true);
      const touch = touched ? [...state.touches, ...state.endedTouches].find((t) => t.market === market && t.touchedAt === touched) : undefined;
      if (touch) return openPattern(touchPattern(touch), target, true);
    } catch {
      // Gone from the list, or the server not answering: the market is still opened below.
    }
  }
  if (market && isChartMarket(market)) {
    batch(() => {
      setPanelMarket(target.id, market);
      if (tf) setPanelTimeframe(target.id, tf);
    });
  }
}
