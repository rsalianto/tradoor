/**
 * What a chart needs for a footprint beyond the main script: the browser's
 * recorder, the exchanges' trade streams and the minute book. Loaded the first
 * time a chart shows a footprint, splits a TPO profile by side, or records a
 * market the server does not; a layout of the server's markets never loads it.
 */
export { MinuteBook, toMinute } from './fp-minutes';
export { listenPrints } from './orderflow';
export { onRecorded, recordedFootprint, recordStatus, want } from './fp-recorder';
