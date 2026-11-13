import { call, type AdaptiveSettings, type AdaptiveView } from "./api";
import { h, s } from "./dom";
import { thinSlider, toggle } from "./controls";
import { kelvinGradient } from "./color";
import { row, stack, unit, value } from "./ui";

const hhmm = (min: number) => { const m = Math.round(min); return `${String(Math.floor(m / 60) % 24).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`; };
const parse = (t: string) => {
  const [a, b] = t.split(":").map(Number);
  return Number.isFinite(a) && Number.isFinite(b) ? a * 60 + b : 0;
};

type SliderKey = "dayTemp" | "nightTemp" | "dayBrightness" | "nightBrightness";

/** `tab` is the compact Auto tab; `settings` holds the rarely used options for the gear sheet. */
export function createAdaptive(): { tab: HTMLElement; settings: HTMLElement; update(v: AdaptiveView): void } {
  let cur: AdaptiveSettings | null = null;
  let timer = 0;

  const push = () => {
    window.clearTimeout(timer);
    timer = 0;
    if (cur) void call("set_adaptive", { settings: cur });
  };
  const edit = (patch: Partial<AdaptiveSettings>) => {
    if (!cur) return;
    cur = { ...cur, ...patch };
    sync(cur);
    window.clearTimeout(timer);
    timer = window.setTimeout(push, 250);
  };

  const power = toggle("Adaptive", (on) => { edit({ enabled: on }); push(); });
  const status = h("small", { "aria-live": "polite" });
  const resume = h("button", { class: "link", type: "button", text: "Resume", onClick: () => void call("adaptive_resume") });
  resume.hidden = true;

  // curve graph
  // Drawn in real pixels: the viewBox width follows the element, so X0/X1 always sit on the row's
  // text edges (1px in for the stroke) and the curve lines up with the labels at any window width.
  const H = 92, X0 = 1, Y0 = 8, Y1 = 70;
  let X1 = 339;
  const area = s("path", { class: "g-area" });
  const line = s("path", { class: "g-line", fill: "none", "stroke-linecap": "round", "stroke-linejoin": "round" });
  const nowLine = s("line", { class: "g-now", "stroke-linecap": "round", y1: Y0, y2: Y1 });
  const nowDot = s("circle", { class: "g-dot", r: 5 });
  const base = s("path", { class: "g-base", "stroke-linecap": "round" });
  const ticks = [0, 6, 12, 18, 24].map((hr) => {
    const t = s("text", { class: "g-tick", y: H - 6, "text-anchor": hr === 0 ? "start" : hr === 24 ? "end" : "middle" });
    t.textContent = String(hr);
    return t;
  });
  const graph = s("svg", { class: "graph", role: "img", "aria-label": "Today's light curve" }, area, base, line, nowLine, nowDot, ...ticks);
  const gx = (m: number) => X0 + (m / 1440) * (X1 - X0);
  let last: AdaptiveView | null = null;
  const draw = () => {
    graph.setAttribute("viewBox", `0 0 ${X1 + 1} ${H}`);
    base.setAttribute("d", `M${X0} ${Y1}H${X1}`);
    ticks.forEach((t, i) => t.setAttribute("x", String(gx(i * 360))));
    const v = last;
    if (!v) return;
    const tN = (t: number) => Y1 - ((t - 2700) / 3800) * (Y1 - Y0 - 6) - 3;
    const bN = (b: number) => Y1 - ((b - 30) / 370) * (Y1 - Y0 - 6) - 3;
    const pts = v.curve;
    line.setAttribute("d", pts.map((p, i) => `${i ? "L" : "M"}${gx(p[0]).toFixed(1)} ${tN(p[1]).toFixed(1)}`).join(""));
    area.setAttribute("d", pts.map((p, i) => `${i ? "L" : "M"}${gx(p[0]).toFixed(1)} ${bN(p[2]).toFixed(1)}`).join("") + `L${X1} ${Y1}L${X0} ${Y1}Z`);
    const nx = gx(v.nowMin);
    nowLine.setAttribute("x1", String(nx));
    nowLine.setAttribute("x2", String(nx));
    nowDot.setAttribute("cx", String(nx));
    nowDot.setAttribute("visibility", pts.length ? "visible" : "hidden");
    if (pts.length) nowDot.setAttribute("cy", String(tN(pts[Math.min(pts.length - 1, Math.round(v.nowMin / 15))][1])));
  };
  draw();
  new ResizeObserver(([e]) => {
    const w = Math.round(e.contentRect.width);
    // a hidden tab reports 0; keep the last real width
    if (w > 2 && w - 1 !== X1) { X1 = w - 1; draw(); }
  }).observe(graph as unknown as Element);

  const sliders: Record<string, { set(v: number): void }> = {};
  const mk = (key: SliderKey, label: string, min: number, max: number, step: number, fmt: (v: number) => string, compact: boolean) => {
    const val = value();
    const t = thinSlider({ label, min, max, step, gradient: key.endsWith("Temp") ? kelvinGradient() : undefined, onInput: (v) => edit({ [key]: v } as Partial<AdaptiveSettings>) });
    sliders[key] = { set: (v) => { t.set(v); val.textContent = fmt(v); } };
    return compact
      ? h("div", { class: "col" }, h("div", { class: "line" }, h("span", { text: label }), val), t.el)
      : stack(label, val, t.el);
  };
  const K = (v: number) => unit(v, "K");
  const lm = (v: number) => unit(v, "lm");

  // ---- tab
  const tab = h("div", { class: "panel-body" },
    h("div", { class: "row" }, h("div", { class: "label" }, h("span", { text: "Match time of day" }), status), resume, power.el),
    h("div", { class: "row graphrow" }, graph as unknown as Node),
    h("div", { class: "row pair" }, mk("dayTemp", "Day", 2700, 6500, 100, K, true), mk("nightTemp", "Night", 2700, 6500, 100, K, true)),
  );

  // ---- sheet settings: two times, two brightnesses, one switch. The rest keeps its defaults.
  const time = (label: string) => {
    const inp = h("input", { class: "field time", type: "time", "aria-label": label }) as HTMLInputElement;
    // a config that follows the sun by coordinates becomes plain clock times once either is edited
    inp.addEventListener("change", () => inp.value && edit({ mode: "fixed", sunriseMin: parse(rise.value), sunsetMin: parse(set.value) }));
    return inp;
  };
  const rise: HTMLInputElement = time("Day starts");
  const set: HTMLInputElement = time("Night starts");
  const back = toggle("Tint back light", (on) => edit({ includeBack: on }));

  const settings = h("div", { class: "adaptive-settings" },
    h("div", { class: "group" }, row("Day starts", rise), row("Night starts", set)),
    h("div", { class: "group" },
      mk("dayBrightness", "Day brightness", 30, 400, 5, lm, false),
      mk("nightBrightness", "Night brightness", 30, 400, 5, lm, false),
    ),
    h("div", { class: "group" }, row("Tint back light", back.el, "The back light warms and cools with the front")),
  );

  function sync(c: AdaptiveSettings) {
    power.set(c.enabled);
    if (c.mode === "fixed" && document.activeElement !== rise && document.activeElement !== set) { rise.value = hhmm(c.sunriseMin); set.value = hhmm(c.sunsetMin); }
    sliders.dayTemp.set(c.dayTemp);
    sliders.nightTemp.set(c.nightTemp);
    sliders.dayBrightness.set(c.dayBrightness);
    sliders.nightBrightness.set(c.nightBrightness);
    back.set(c.includeBack);
  }

  return {
    tab,
    settings,
    update(v) {
      if (!timer) {
        cur = v.settings;
        sync(cur);
      }
      const c = cur ?? v.settings;
      const [event = "", at] = v.next.split(" ");
      const next = event ? ` · ${event[0].toUpperCase()}${event.slice(1)}${at ? ` at ${at}` : ""}` : "";
      status.textContent = v.pausedText ? `Paused until ${v.pausedText}`
        : c.enabled ? `${unit(v.targetTemp, "K")}, ${unit(v.targetBrightness, "lm")}${next}`
        : "Off";
      resume.hidden = !v.pausedText;
      // following the sun: show today's real times
      if (c.mode === "sun" && document.activeElement !== rise && document.activeElement !== set) { rise.value = hhmm(v.sunrise); set.value = hhmm(v.sunset); }
      last = v;
      draw();
      graph.classList.toggle("off", !c.enabled);
    },
  };
}
