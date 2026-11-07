/** A point of a chart, in the chart's own units. */
export interface Point {
  x: number;
  y: number;
}

const short = (v: number): number => Number(v.toFixed(3));

/**
 * SVG path data through every point, as a curve that rounds each turn but never
 * overshoots: between two points it stays between their values, so it shows no
 * high or low that was not reached (a monotone cubic, Steffen's, as d3's
 * curveMonotoneX). Two points at the same x are joined straight.
 */
export function smoothPath(points: readonly Point[]): string {
  if (points.length === 0) return '';
  const last = points.length - 1;
  const h = points.slice(1).map((p, i) => p.x - points[i]!.x);
  const s = points.slice(1).map((p, i) => (h[i]! > 0 ? (p.y - points[i]!.y) / h[i]! : 0));
  // Each point's lean: none where the way turns or meets a jump, else the gentler side's.
  const t = points.map((_, i) => {
    if (i === 0 || i === last) return 0;
    const [h0, h1, s0, s1] = [h[i - 1]!, h[i]!, s[i - 1]!, s[i]!];
    if (h0 <= 0 || h1 <= 0) return 0;
    const p = (s0 * h1 + s1 * h0) / (h0 + h1);
    return (Math.sign(s0) + Math.sign(s1)) * Math.min(Math.abs(s0), Math.abs(s1), 0.5 * Math.abs(p));
  });
  // The ends lean the way their one segment goes, eased by the point next to them.
  if (last > 0) {
    t[0] = h[0]! > 0 ? (3 * s[0]! - (last > 1 ? t[1]! : s[0]!)) / 2 : 0;
    t[last] = h[last - 1]! > 0 ? (3 * s[last - 1]! - (last > 1 ? t[last - 1]! : s[last - 1]!)) / 2 : 0;
  }
  let d = `M${short(points[0]!.x)},${short(points[0]!.y)}`;
  for (let i = 0; i < last; i++) {
    const [a, b] = [points[i]!, points[i + 1]!];
    if (h[i]! <= 0) {
      d += `L${short(b.x)},${short(b.y)}`;
      continue;
    }
    const third = h[i]! / 3;
    d += `C${short(a.x + third)},${short(a.y + third * t[i]!)} ${short(b.x - third)},${short(b.y - third * t[i + 1]!)} ${short(b.x)},${short(b.y)}`;
  }
  return d;
}
