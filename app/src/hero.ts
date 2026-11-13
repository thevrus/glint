import { s, h } from "./dom";
import { css, hexToRgb, kelvinRgb, mix, tween, type RGB } from "./color";

export interface HeroState {
  on: boolean;
  brightness: number;
  temperature: number;
  hasBack: boolean;
  backOn: boolean;
  backBrightness: number;
  zones: string[];
  /** 1..7 selects one zone, 0 selects all, -1 none */
  selected: number;
}

const OFF_PANEL: RGB = [44, 42, 40];
const DARK: RGB = [28, 28, 30];
const SEG_X = 52, SEG_W = (256 - 6 * 4) / 7, SEG_GAP = 4;
const segX = (i: number) => SEG_X + i * (SEG_W + SEG_GAP);

interface V { panel: RGB; lit: number; segs: RGB[]; rake: number[]; selX: number; selW: number; selOn: number }

export function createHero(onZone: (zone: number) => void, onPanel: () => void): { el: HTMLElement; update(st: HeroState, instant?: boolean): void } {
  const defs = s("defs");
  const grad = (id: string, x2: number, y2: number, stops: [number, string, number][]) => {
    const g = s("linearGradient", { id, x1: 0, y1: 0, x2, y2 });
    for (const [o, c, a] of stops) g.append(s("stop", { offset: o, "stop-color": c, "stop-opacity": a }));
    return g;
  };
  defs.append(
    grad("hv", 0, 1, [[0, "#fff", 0.26], [0.35, "#fff", 0.04], [1, "#000", 0.16]]),
    grad("hh", 1, 0, [[0, "#000", 0.2], [0.22, "#000", 0], [0.78, "#000", 0], [1, "#000", 0.2]]),
    grad("sheen", 1, 1, [[0, "#fff", 0], [0.45, "#fff", 0.16], [0.55, "#fff", 0.04], [1, "#fff", 0]]),
    grad("bezel", 0, 1, [[0, "#4a4947", 1], [1, "#2e2d2c", 1]]),
  );
  const dots = s("pattern", { id: "dots", width: 5, height: 5, patternUnits: "userSpaceOnUse" });
  dots.append(s("circle", { cx: 2.5, cy: 2.5, r: 0.7, fill: "#000", "fill-opacity": 0.1 }));
  defs.append(dots);
  const clip = s("clipPath", { id: "dclip" });
  clip.append(s("rect", { x: 78, y: 36, width: 204, height: 92, rx: 8 }));
  defs.append(clip);
  const rakeGrads: SVGLinearGradientElement[] = [];
  for (let i = 0; i < 7; i++) {
    const g = grad(`rk${i}`, 0, 1, [[0, "#fff", 0.5], [1, "#fff", 0]]);
    rakeGrads.push(g);
    defs.append(g);
  }

  const panelFill = s("rect", { x: 78, y: 36, width: 204, height: 92, rx: 8 });
  const litLayers = s("g", { "clip-path": "url(#dclip)" },
    s("rect", { x: 78, y: 36, width: 204, height: 92, fill: "url(#hv)" }),
    s("rect", { x: 78, y: 36, width: 204, height: 92, fill: "url(#hh)" }),
    s("rect", { x: 78, y: 36, width: 204, height: 92, fill: "url(#dots)" }),
    s("rect", { x: 40, y: 20, width: 90, height: 150, fill: "url(#sheen)", transform: "rotate(24 180 80) translate(86 -20)" }),
  );
  const panelGroup = s("g", { class: "hero-panel", role: "button", tabindex: 0, "aria-label": "Front light" },
    s("rect", { x: 64, y: 22, width: 232, height: 120, rx: 16, fill: "url(#bezel)" }),
    s("rect", { x: 64.5, y: 22.5, width: 231, height: 119, rx: 15.5, fill: "none", stroke: "#fff", "stroke-opacity": 0.14 }),
    s("rect", { x: 76, y: 34, width: 208, height: 96, rx: 10, fill: "#1c1b1a" }),
    panelFill,
    litLayers,
  );
  panelGroup.addEventListener("click", onPanel);

  const neck = s("g", {},
    s("rect", { x: 172, y: 140, width: 16, height: 20, fill: "#2c2b2a" }),
    s("rect", { x: 172, y: 140, width: 3, height: 20, fill: "#fff", "fill-opacity": 0.07 }),
    s("circle", { cx: 180, cy: 144, r: 7.5, fill: "#3b3a38", stroke: "#fff", "stroke-opacity": 0.12 }),
    s("circle", { cx: 180, cy: 144, r: 2.2, fill: "#1c1b1a" }),
  );
  const housing = s("rect", { x: 44, y: 158, width: 272, height: 30, rx: 10, fill: "#2c2b2a", stroke: "#fff", "stroke-opacity": 0.1 });
  const rakes: SVGRectElement[] = [];
  const segs: SVGRectElement[] = [];
  const zonesG = s("g", {});
  for (let i = 0; i < 7; i++) {
    const rake = s("rect", { x: segX(i), y: 190, width: SEG_W, height: 24, fill: `url(#rk${i})` });
    rakes.push(rake);
    zonesG.append(rake);
  }
  const sel = s("rect", { y: 160, height: 26, rx: 8, fill: "none", stroke: "#fff", "stroke-width": 2, "pointer-events": "none" });
  for (let i = 0; i < 7; i++) {
    const seg = s("rect", { x: segX(i), y: 164, width: SEG_W, height: 18, rx: 5 });
    segs.push(seg);
    const hit = s("rect", { x: segX(i) - 1, y: 160, width: SEG_W + 2, height: 54, fill: "transparent", class: "hero-zone", role: "button", tabindex: 0, "aria-label": `Back zone ${i + 1}` });
    const tip = s("title"); tip.textContent = `Zone ${i + 1}`; hit.append(tip);
    hit.addEventListener("click", () => onZone(i + 1));
    hit.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onZone(i + 1)));
    zonesG.append(seg, hit);
  }
  const back = s("g", { class: "hero-back" }, housing, zonesG, sel);

  const svg = s("svg", { viewBox: "0 0 360 220", role: "img", "aria-label": "Your light, live", class: "hero-svg" }, defs, s("g", {}, back, neck, panelGroup));
  panelGroup.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onPanel()));
  const el = h("div", { class: "hero" }, svg as unknown as Node);

  let v: V = { panel: OFF_PANEL, lit: 0, segs: Array(7).fill(DARK), rake: Array(7).fill(0), selX: segX(0), selW: SEG_W, selOn: 0 };
  let cancel = () => {};
  let goal = "";
  let ring = { x: segX(0), w: SEG_W };
  const render = () => {
    panelFill.setAttribute("fill", css(v.panel));
    litLayers.setAttribute("opacity", String(0.25 + 0.75 * v.lit));
    segs.forEach((r, i) => r.setAttribute("fill", css(v.segs[i])));
    rakeGrads.forEach((g, i) => {
      const c = css(v.segs[i]);
      const [a, b] = g.children;
      a.setAttribute("stop-color", c);
      a.setAttribute("stop-opacity", String(v.rake[i]));
      b.setAttribute("stop-color", c);
    });
    sel.setAttribute("x", String(v.selX - 3));
    sel.setAttribute("width", String(v.selW + 6));
    sel.setAttribute("opacity", String(v.selOn));
  };
  render();

  return {
    el,
    update(st, instant = false) {
      const frac = Math.min(1, Math.max(0, (st.brightness - 30) / 370));
      const lit = st.on ? 0.18 + 0.82 * Math.pow(frac, 0.65) : 0;
      const target: V = {
        panel: mix(OFF_PANEL, kelvinRgb(st.temperature), lit),
        lit,
        segs: [],
        rake: [],
        // with nothing selected the ring fades out where it stands
        selX: st.selected < 0 ? ring.x : st.selected === 0 ? segX(0) : segX(st.selected - 1),
        selW: st.selected < 0 ? ring.w : st.selected === 0 ? segX(6) + SEG_W - segX(0) : SEG_W,
        selOn: st.hasBack && st.selected >= 0 ? 1 : 0,
      };
      ring = { x: target.selX, w: target.selW };
      const bi = Math.min(1, st.backBrightness / 100);
      for (let i = 0; i < 7; i++) {
        const z = hexToRgb(st.zones[i] ?? "#ffffff");
        const on = st.hasBack && st.backOn;
        target.segs.push(on ? mix(DARK, z, 0.3 + 0.7 * bi) : mix(z, DARK, 0.86));
        target.rake.push(on ? 0.12 + 0.5 * bi : 0);
      }
      back.setAttribute("display", st.hasBack ? "inline" : "none");
      // The backend echoes every write back as a state event; an update that asks for the
      // picture already shown or already being eased to must not restart the tween.
      const key = JSON.stringify(target);
      if (key === goal && !instant) return;
      goal = key;
      cancel();
      // Under the pointer the picture tracks the control exactly; easing would trail it.
      if (instant) { cancel = () => {}; v = target; render(); return; }
      const from = v;
      const L = (a: number, b: number, t: number) => a + (b - a) * t;
      cancel = tween(250, (t) => {
        v = {
          panel: mix(from.panel, target.panel, t),
          lit: L(from.lit, target.lit, t),
          segs: target.segs.map((c, i) => mix(from.segs[i], c, t)),
          rake: target.rake.map((r, i) => L(from.rake[i], r, t)),
          selX: L(from.selX, target.selX, t),
          selW: L(from.selW, target.selW, t),
          selOn: L(from.selOn, target.selOn, t),
        };
        render();
      });
    },
  };
}
