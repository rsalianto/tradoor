/**
 * A chart picture's title, in its top-left corner where the legend stands on
 * screen: the market, its timeframe and exchange, the bar's prices, and when
 * the picture was taken — a picture passed on says what it is of. Drawn on
 * the page, in its own font and colours, and only then made a PNG. Loaded
 * with the first picture.
 */
import type { CaptureTitle } from './capture';

/** A theme colour as the canvas can take it: the variable worked out on an element. */
function themed(name: string, fallback: string): string {
  const probe = document.createElement('span');
  probe.style.color = `var(${name}, ${fallback})`;
  document.body.append(probe);
  const color = getComputedStyle(probe).color;
  probe.remove();
  return color || fallback;
}

export function titled({ picture, px }: { picture: ImageBitmap; px: number }, title: CaptureTitle): Promise<Blob> {
  const canvas = document.createElement('canvas');
  canvas.width = picture.width;
  canvas.height = picture.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('this browser will not draw the picture');
  ctx.drawImage(picture, 0, 0);
  picture.close();
  ctx.scale(px, px);
  const family = getComputedStyle(document.body).fontFamily || 'monospace';
  const text = themed('--text', '#dfe6e0');
  // Muted and dim are the text over the surface, as the page mixes them: drawn faded over the box.
  type Run = { text: string; size: number; bold?: boolean; color: string; alpha?: number };
  const lines: Run[][] = [
    [
      { text: title.symbol, size: 15, bold: true, color: text },
      { text: `  ${title.detail}`, size: 12, color: text, alpha: 0.6 },
    ],
    ...(title.ohlc
      ? [
          [
            { text: title.ohlc, size: 11.5, color: text, alpha: 0.6 },
            { text: ` ${title.change}`, size: 11.5, color: themed(title.up ? '--bull' : '--bear', title.up ? '#aafd35' : '#ff4d5f') },
          ],
        ]
      : []),
    [{ text: title.time, size: 11, color: text, alpha: 0.42 }],
  ];
  const font = (run: Run) => `${run.bold ? 700 : 400} ${run.size}px ${family}`;
  const heights = lines.map((line) => Math.max(...line.map((r) => r.size)) * 1.35);
  const width = Math.max(
    ...lines.map((line) =>
      line.reduce((sum, run) => {
        ctx.font = font(run);
        return sum + ctx.measureText(run.text).width;
      }, 0),
    ),
  );
  const [x, y, padX, padY] = [8, 8, 10, 7];
  const box = { w: width + padX * 2, h: heights.reduce((a, b) => a + b, 0) + padY * 2 };
  ctx.beginPath();
  ctx.roundRect(x, y, box.w, box.h, 5);
  ctx.globalAlpha = 0.9;
  ctx.fillStyle = themed('--surface', '#0d100e');
  ctx.fill();
  ctx.globalAlpha = 1;
  ctx.lineWidth = 1;
  ctx.strokeStyle = themed('--border', '#232823');
  ctx.stroke();
  ctx.textBaseline = 'middle';
  let top = y + padY;
  lines.forEach((line, i) => {
    let left = x + padX;
    const middle = top + heights[i]! / 2;
    for (const run of line) {
      ctx.font = font(run);
      ctx.fillStyle = run.color;
      ctx.globalAlpha = run.alpha ?? 1;
      ctx.fillText(run.text, left, middle);
      left += ctx.measureText(run.text).width;
    }
    top += heights[i]!;
  });
  return new Promise((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('the picture was not made'))), 'image/png'));
}
