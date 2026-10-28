/** Stroke icons, drawn on a 24-unit grid in the current text colour. */

const PATHS = {
  pointer: 'M6 3l12 9-5.5 1L10 20z',
  path: 'M3 18l5-8 5 5 8-10',
  trendline: 'M4 20L20 4',
  horizontal: 'M3 12h18',
  ray: 'M5 12h16M17 8l4 4-4 4',
  rect: 'M4 6h16v12H4z',
  fib: 'M3 5h18M3 9.5h18M3 12h18M3 14.5h18M3 19h18',
  channel: 'M3 15L15 5M9 19L21 9',
  magnet: 'M5 4h4v8a3 3 0 0 0 6 0V4h4v8a7 7 0 0 1-14 0zM5 8h4M15 8h4',
  text: 'M5 5h14M12 5v14M9 19h6',
  measure: 'M4 17L17 4l3 3L7 20zM8 13l2 2M11 10l2 2M14 7l2 2',
  long: 'M12 20V5M6 11l6-6 6 6',
  short: 'M12 4v15M6 13l6 6 6-6',
  brush: 'M4 20c3 0 4-1.5 4-4a3 3 0 0 1 3-3M11 13l8-8 1.5 1.5-8 8z',
  trash: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13',
  eraser: 'M9 20h11M4 15l9-9 6 6-8 8H9z',
  eye: 'M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12zM12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z',
  eyeOff: 'M3 3l18 18M10.6 6.1A10 10 0 0 1 12 6c6.5 0 10 6 10 6a17 17 0 0 1-3 3.6M6.5 7.6C3.8 9.2 2 12 2 12s3.5 6 10 6a9.7 9.7 0 0 0 4.5-1.1',
  close: 'M6 6l12 12M18 6L6 18',
  menu: 'M3 6h18M3 12h18M3 18h18',
  plus: 'M12 5v14M5 12h14',
  section: 'M4 6h16M4 6v12M9 12h11M9 18h11',
  search: 'M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM20 20l-4.8-4.8',
  chevron: 'M6 9l6 6 6-6',
  splitRight: 'M4 5h16v14H4zM12 5v14',
  splitDown: 'M4 5h16v14H4zM4 12h16',
  layout: 'M4 5h16v14H4zM12 5v14M4 12h16',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9L7 7M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1',
  command: 'M4 7l5 5-5 5M11 17h9',
  indicators: 'M3 17l5-6 4 3 8-9M3 21h18',
  reset: 'M4 12a8 8 0 1 0 2.3-5.7M4 4v5h5',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  server: 'M4 4h16v6H4zM4 14h16v6H4zM7 7h4M7 17h4',
  up: 'M6 15l6-6 6 6',
  alarm: 'M12 21a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM12 9v4l2.5 2M3.5 6.5l3-3M20.5 6.5l-3-3',
  camera: 'M3 8h3.5L8 5.5h8L17.5 8H21v11H3zM12 16.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z',
  info: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v6M12 7.5v.5',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  pencil: 'M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4',
  undo: 'M9 14L4 9l5-5M4 9h10.5a5.5 5.5 0 0 1 0 11H11',
  redo: 'M15 14l5-5-5-5M20 9H9.5a5.5 5.5 0 0 0 0 11H13',
  sort: 'M7 4v16M4 17l3 3 3-3M14 6h6M14 11h4M14 16h2',
  down: 'M6 9l6 6 6-6',
  gift: 'M4 11h16v9H4zM3 7h18v4H3zM12 7v13M12 7C11 4.5 8.5 3 7.2 4.3S8.5 7 12 7zM12 7c1-2.5 3.5-4 4.8-2.7S15.5 7 12 7z',
  external: 'M14 4h6v6M20 4l-9 9M18 14v6H4V6h6',
  star: 'M12 3.8l2.5 5.2 5.7.8-4.1 4 1 5.6-5.1-2.7-5.1 2.7 1-5.6-4.1-4 5.7-.8z',
  user: 'M20 21v-1.5a4.5 4.5 0 0 0-4.5-4.5h-7A4.5 4.5 0 0 0 4 19.5V21M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
  send: 'M21 3L10 14M21 3l-6.5 18-4.5-7-7-4.5z',
  droplet: 'M12 3l5.5 5.5a7.8 7.8 0 1 1-11 0z',
} as const;

export type IconName = keyof typeof PATHS;

export function Icon(props: { name: IconName; size?: number; stroke?: number; fill?: boolean }) {
  return (
    <svg
      width={props.size ?? 14}
      height={props.size ?? 14}
      viewBox="0 0 24 24"
      fill={props.fill ? 'currentColor' : 'none'}
      stroke="currentColor"
      stroke-width={props.stroke ?? 1.8}
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[props.name]} />
    </svg>
  );
}
