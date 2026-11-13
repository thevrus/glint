type Kid = Node | string | null | false | undefined;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, unknown> = {},
  ...kids: Kid[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === false || v == null) continue;
    if (k === "class") el.className = String(v);
    else if (k === "text") el.textContent = String(v);
    else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2).toLowerCase(), v as EventListener);
    else el.setAttribute(k, v === true ? "" : String(v));
  }
  for (const kid of kids) if (kid) el.append(kid);
  return el;
}

const NS = "http://www.w3.org/2000/svg";
export function s<K extends keyof SVGElementTagNameMap>(
  tag: K,
  props: Record<string, string | number> = {},
  ...kids: (SVGElement | null)[]
): SVGElementTagNameMap[K] {
  const el = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(props)) el.setAttribute(k, String(v));
  for (const kid of kids) if (kid) el.append(kid);
  return el;
}

// One set, one grid: a 20 px box with the drawing kept inside 2.75..17.25, a 1.5 stroke with
// round caps and joins, 2 px corners, and lines that stop short of a knob instead of running
// through it. No fills except the dots.
const G: Record<string, string> = {
  sun: '<circle cx="10" cy="10" r="3"/><path d="M10 2.75V4.5M10 15.5v1.75M2.75 10H4.5M15.5 10h1.75M4.87 4.87l1.24 1.24M13.89 13.89l1.24 1.24M4.87 15.13l1.24-1.24M13.89 6.11l1.24-1.24"/>',
  bulb: '<path d="M7.5 13.25A5.25 5.25 0 1 1 12.5 13.25V14.5h-5z"/><path d="M8.25 17.25h3.5"/>',
  bulbslash: '<path d="M7.5 13.25A5.25 5.25 0 1 1 12.5 13.25V14.5h-5z"/><path d="M8.25 17.25h3.5M3.25 2.75l13.5 14.5"/>',
  plus: '<path d="M10 4.25v11.5M4.25 10h11.5"/>',
  power: '<path d="M10 2.75v6.5M6 5.4a6.25 6.25 0 1 0 8 0"/>',
  trash: '<path d="M4.25 5.75h11.5M7.75 5.75V4.5c0-.69.56-1.25 1.25-1.25h2c.69 0 1.25.56 1.25 1.25v1.25M5.75 5.75l.6 9.6a1.5 1.5 0 0 0 1.5 1.4h4.3a1.5 1.5 0 0 0 1.5-1.4l.6-9.6"/>',
  moon: '<path d="M17 10.6A6.8 6.8 0 1 1 9.4 3a5.5 5.5 0 0 0 7.6 7.6z"/>',
  video: '<rect x="2.75" y="5.75" width="9.5" height="8.5" rx="2"/><path d="m12.25 8.75 5-2.5v7.5l-5-2.5"/>',
  focus: '<circle cx="10" cy="10" r="7.25"/><circle cx="10" cy="10" r="3.25"/><circle cx="10" cy="10" r=".9" fill="currentColor" stroke="none"/>',
  sunset: '<path d="M2.75 14.25h14.5M5.75 14.25a4.25 4.25 0 0 1 8.5 0M10 4.75v2M4.4 7.9l1.4 1.4M15.6 7.9l-1.4 1.4M7 17.25h6"/>',
  slider: '<path d="M2.75 6.25H5M9 6.25h8.25M2.75 13.75H11M15 13.75h2.25"/><circle cx="7" cy="6.25" r="2"/><circle cx="13" cy="13.75" r="2"/>',
  more: '<circle cx="4.5" cy="10" r="1.25" fill="currentColor" stroke="none"/><circle cx="10" cy="10" r="1.25" fill="currentColor" stroke="none"/><circle cx="15.5" cy="10" r="1.25" fill="currentColor" stroke="none"/>',
};

export function icon(name: string, size = 20, stroke = 1.5): HTMLSpanElement {
  const el = document.createElement("span");
  el.className = "ic";
  el.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${G[name] ?? G.slider}</svg>`;
  return el;
}
