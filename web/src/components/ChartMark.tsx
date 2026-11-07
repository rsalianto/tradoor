/**
 * The tradoor mark over a live chart, bottom left of its price pane, where the
 * engine leaves it to the page (`set_page_mark`): every ten seconds its door
 * swings shut over the doorway and open again, the candles behind it moved on
 * by then, a few seconds' play with nothing running between, and a click opens
 * the partners' rewards. A picture of the chart still carries the engine's own,
 * standing still.
 */
import { onCleanup, onMount } from 'solid-js';
import { setOverlay } from '../app/ui';

const EVERY_MS = 10_000;
const PLAY_MS = 3_600;
/** Every mark on the page plays at once, from one clock. */
const marks = new Set<() => void>();
let clock: ReturnType<typeof setInterval> | undefined;
const still = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
// Where each part turns from, and the shut door's rest out of sight, held by the parts themselves.
const HINGE = 'transform-box: fill-box; transform-origin: left center';
const FOOT = 'transform-box: fill-box; transform-origin: center bottom';
const SHUT = 'transform-box: fill-box; transform-origin: right center; transform: scaleX(0)';

export function ChartMark(props: { x: number; y: number; accent: string; bear: string; paused: boolean }) {
  let door: SVGPathElement | undefined;
  let shut: SVGGElement | undefined;
  let rise: SVGRectElement | undefined;
  let fall: SVGRectElement | undefined;
  /** Keyframes of one transform, `[value, at]`, each eased into the next. */
  const keys = (fn: string, steps: [number, number][]): Keyframe[] => steps.map(([v, offset]) => ({ transform: `${fn}(${v})`, offset, easing: 'ease-in-out' }));
  const play = () => {
    if (props.paused || document.hidden || still() || !door || !shut || !rise || !fall) return;
    // One play at a time: what is left of the last goes, so nothing can hold the door shut.
    for (const el of [door, shut, rise, fall]) for (const a of el.getAnimations()) a.cancel();
    const timing = { duration: PLAY_MS };
    // The open door turns edge on; the closed one swings across the doorway from the
    // hinge, stays a moment, and swings back; then the open door comes round again.
    door.animate(keys('scaleX', [[1, 0], [0, 0.18], [0, 0.68], [1, 0.86], [1, 1]]), timing);
    shut.animate(keys('scaleX', [[0, 0], [0, 0.18], [1, 0.34], [1, 0.52], [0, 0.68], [0, 1]]), timing);
    // Behind it, the market moved: the candles are not where they were.
    rise.animate(keys('scaleY', [[1, 0], [1, 0.68], [1.6, 0.82], [0.8, 0.92], [1, 1]]), timing);
    fall.animate(keys('scaleY', [[1, 0], [1, 0.68], [0.5, 0.82], [1.25, 0.92], [1, 1]]), timing);
  };
  onMount(() => {
    marks.add(play);
    clock ??= setInterval(() => marks.forEach((p) => p()), EVERY_MS);
    onCleanup(() => {
      marks.delete(play);
      if (marks.size === 0) {
        clearInterval(clock);
        clock = undefined;
      }
    });
  });
  return (
    <button
      class="chart-mark"
      style={{ left: `${props.x}px`, top: `${props.y}px` }}
      aria-label="Rewards from tradoor's partners"
      data-tip="Rewards"
      onPointerDown={(e) => e.stopPropagation()}
      onClick={() => setOverlay({ kind: 'referrals' })}
    >
      {/* The engine's mark on the icon's sixteen-unit grid (tradoor-render chart, `paint_watermark`). */}
      <svg viewBox="0 0 16 16" width="34" height="34" aria-hidden="true">
        <rect x="1.8" y="3" width="7.6" height="10" fill="none" stroke={props.accent} stroke-width="1.5" opacity="0.59" />
        <rect ref={rise} style={FOOT} x="3.3" y="7.6" width="1.7" height="3.4" fill={props.accent} opacity="0.8" />
        <rect ref={fall} style={FOOT} x="6.2" y="5.6" width="1.7" height="5.2" fill={props.bear} opacity="0.8" />
        {/* The door shut: across the doorway, hung on its right edge, out of sight until it swings. */}
        <g ref={shut} style={SHUT}>
          <rect x="1.8" y="3" width="7.6" height="10" fill={props.accent} stroke={props.accent} stroke-width="1.5" stroke-linejoin="round" />
          <circle cx="3.5" cy="8.3" r="0.75" fill="#000" fill-opacity="0.55" />
        </g>
        <path ref={door} style={HINGE} d="M9.4 3 L13.6 1.2 L13.6 14.8 L9.4 13 Z" fill="none" stroke={props.accent} stroke-width="1.5" stroke-linejoin="round" opacity="0.69" />
      </svg>
    </button>
  );
}
