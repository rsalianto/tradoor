/**
 * Exchanges and their marks. Those `live` can be charted — they have venues,
 * streams and history loaders.
 * Marks are the official ones, on a black disc, from rsalianto's exchange icons.
 */
import { Show, type Component, type JSX } from 'solid-js';
import { Dynamic, For } from 'solid-js/web';
import { MARKS, type ExchangeMark } from './exchange-marks';
import { parseMarket, ratioLegs, venueLabel, type Venue } from './markets';

export type ExchangeId = 'binance' | 'bybit' | 'okx' | 'bitget' | 'hyperliquid' | 'kraken' | 'lighter' | 'index' | 'gate' | 'htx' | 'bitfinex' | 'aster' | 'deribit' | 'coinbase' | 'bitstamp';

interface LogoProps {
  size?: number;
}

export interface ExchangeDef {
  id: ExchangeId;
  name: string;
  /** Venues this app can chart, stream and store. */
  live: boolean;
  Logo: Component<LogoProps>;
}

function Mark(props: LogoProps & { viewBox: string; children: JSX.Element }) {
  const size = () => props.size ?? 14;
  return (
    <svg class="xlogo" width={size()} height={size()} viewBox={props.viewBox} aria-hidden="true">
      {props.children}
    </svg>
  );
}

/** A mark whose paths the chart engine draws too — one source of artwork. */
function Paths(props: LogoProps & { mark: ExchangeMark }) {
  return (
    <Mark size={props.size} viewBox={`0 0 ${props.mark.box} ${props.mark.box}`}>
      <For each={props.mark.paths}>{(path) => <path d={path.d} fill={path.fill} transform={path.t ? `translate(${path.t[0]} ${path.t[1]}) scale(${path.t[2]})` : undefined} />}</For>
    </Mark>
  );
}

/** Every mark is path data the engine draws too — one source of artwork. */
const logo = (id: ExchangeId): Component<LogoProps> => (p) => <Paths size={p.size} mark={MARKS[id]} />;

export const EXCHANGES: Record<ExchangeId, ExchangeDef> = {
  binance: { id: 'binance', name: 'Binance', live: true, Logo: logo('binance') },
  bybit: { id: 'bybit', name: 'Bybit', live: true, Logo: logo('bybit') },
  okx: { id: 'okx', name: 'OKX', live: true, Logo: logo('okx') },
  bitget: { id: 'bitget', name: 'Bitget', live: true, Logo: logo('bitget') },
  hyperliquid: { id: 'hyperliquid', name: 'Hyperliquid', live: true, Logo: logo('hyperliquid') },
  kraken: { id: 'kraken', name: 'Kraken', live: true, Logo: logo('kraken') },
  lighter: { id: 'lighter', name: 'Lighter', live: true, Logo: logo('lighter') },
  index: { id: 'index', name: 'Index', live: true, Logo: logo('index') },
  gate: { id: 'gate', name: 'Gate', live: true, Logo: logo('gate') },
  htx: { id: 'htx', name: 'HTX', live: true, Logo: logo('htx') },
  bitfinex: { id: 'bitfinex', name: 'Bitfinex', live: true, Logo: logo('bitfinex') },
  aster: { id: 'aster', name: 'Aster', live: true, Logo: logo('aster') },
  deribit: { id: 'deribit', name: 'Deribit', live: true, Logo: logo('deribit') },
  coinbase: { id: 'coinbase', name: 'Coinbase', live: true, Logo: logo('coinbase') },
  bitstamp: { id: 'bitstamp', name: 'Bitstamp', live: true, Logo: logo('bitstamp') },
};

export const ALL_EXCHANGES: readonly ExchangeDef[] = Object.values(EXCHANGES);
export const LIVE_EXCHANGES: readonly ExchangeDef[] = ALL_EXCHANGES.filter((e) => e.live);

export const exchangeOfVenue = (venue: Venue | string): ExchangeDef => EXCHANGES[venue.split('-')[0] as ExchangeId] ?? EXCHANGES.binance;

export function ExchangeLogo(props: { exchange: ExchangeDef; size?: number }) {
  return <Dynamic component={props.exchange.Logo} size={props.size} />;
}

/** PERP, SPOT or INDEX: what a market is, as its row says it. */
export const kindLabel = (market: string): string => {
  const venue = parseMarket(market)?.venue;
  return venue === 'index' ? 'INDEX' : venue?.endsWith('perp') ? 'PERP' : 'SPOT';
};

/** Logo, optionally the exchange's name, and its kind — for a ratio, each
 *  of its markets' logos either side of the stroke. */
export function VenueTag(props: { market: string; name?: boolean; size?: number }) {
  const legs = () => ratioLegs(props.market);
  return (
    <Show when={legs()} fallback={<OneVenue market={props.market} name={props.name} size={props.size} />}>
      {(l) => (
        <span class="venue ratio" data-tip={venueLabel(props.market)}>
          <OneVenue market={l()[0]} size={props.size} />
          <span class="dim">/</span>
          <OneVenue market={l()[1]} size={props.size} />
        </span>
      )}
    </Show>
  );
}

function OneVenue(props: { market: string; name?: boolean; size?: number }) {
  const ref = () => parseMarket(props.market);
  const exchange = () => exchangeOfVenue(ref()?.venue ?? 'binance');
  return (
    <span class="venue" data-tip={ref() ? (ref()!.venue === 'index' ? 'Worked out by the tradoor server' : `${exchange().name} ${ref()!.venue.endsWith('perp') ? 'perpetual' : 'spot'}`) : undefined}>
      <ExchangeLogo exchange={exchange()} size={props.size} />
      {props.name ? <span class="venue-name">{exchange().name}</span> : null}
      <span class="kind">{kindLabel(props.market)}</span>
    </span>
  );
}
