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

// Simple 20pt glyphs in one style: 1.6 stroke, round caps, no fill unless noted.
const G: Record<string, string> = {
  sun: '<circle cx="10" cy="10" r="3.6"/><path d="M10 2.6v2M10 15.4v2M2.6 10h2M15.4 10h2M4.8 4.8l1.4 1.4M13.8 13.8l1.4 1.4M4.8 15.2l1.4-1.4M13.8 6.2l1.4-1.4"/>',
  bulb: '<path d="M6.6 12.4c-1.3-1-2.1-2.5-2.1-4.2a5.5 5.5 0 0 1 11 0c0 1.7-.8 3.2-2.1 4.2-.5.4-.8 1-.8 1.6v.4H7.4V14c0-.6-.3-1.2-.8-1.600z"/><path d="M7.8 17h4.4"/>',
  power: '<path d="M10 2.8v6.4"/><path d="M5.6 5.6a6 6 0 1 0 8.8 0"/>',
  plus: '<path d="M10 4.5v11M4.5 10h11"/>',
  trash: '<path d="M4 5.6h12M8 5.6V4a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v1.6M5.4 5.6l.7 10a1.3 1.3 0 0 0 1.3 1.2h5.2a1.3 1.3 0 0 0 1.3-1.200l.7-10"/>',
  moon: '<path d="M16.2 12.2A6.6 6.6 0 0 1 7.8 3.800 6.6 6.6 0 1 0 16.200 12.200z"/>',
  video: '<rect x="2.6" y="5.2" width="10.4" height="9.6" rx="2.400"/><path d="m13 8.800 4.400-2.400v7.200L13 11.200"/>',
  focus: '<circle cx="10" cy="10" r="6.800"/><circle cx="10" cy="10" r="3"/><circle cx="10" cy="10" r=".6"/>',
  sunset: '<path d="M3 14h14M6 14a4 4 0 0 1 8 0M10 4v2.600M4.400 7.600l1.400 1.400M15.600 7.600 14.200 9M7 17h6"/>',
  slider: '<path d="M3.500 6h13M3.500 14h13"/><circle cx="7" cy="6" r="1.900"/><circle cx="13" cy="14" r="1.900"/>',
  more: '<circle cx="4.500" cy="10" r="1.200" fill="currentColor"/><circle cx="10" cy="10" r="1.200" fill="currentColor"/><circle cx="15.500" cy="10" r="1.200" fill="currentColor"/>',
  pencil: '<path d="m12.600 4.400 3 3L7.400 15.600 3.800 16.200l.6-3.600z"/>',
  bulbslash: '<path d="M6.600 12.400c-1.300-1-2.100-2.500-2.100-4.200a5.500 5.500 0 0 1 11 0c0 1.700-.8 3.200-2.100 4.200-.5.400-.8 1-.8 1.600v.4H7.400V14c0-.6-.3-1.200-.8-1.600z"/><path d="M7.800 17h4.400M3 3l14 14"/>',
};

export function icon(name: string, size = 20, stroke = 1.6): HTMLSpanElement {
  const el = document.createElement("span");
  el.className = "ic";
  el.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${G[name] ?? G.slider}</svg>`;
  return el;
}
