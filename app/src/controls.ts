import { h, icon } from "./dom";
import { hexToRgb, hsvToRgb, rgbToHex, rgbToHsv, css, tween } from "./color";

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));
/** Slider thumb diameter; keep in step with --thumb in style.css. */
const THUMB = 20;

/** One pointer-drag gesture: capture the pointer, measure the element once at pointerdown
 *  and hand that rect to every move, so a drag never forces layout per pointermove. */
function dragTrack(el: HTMLElement, onPoint: (e: PointerEvent, r: DOMRect) => void) {
  let rect: DOMRect;
  el.addEventListener("pointerdown", (e) => {
    el.setPointerCapture(e.pointerId);
    el.classList.add("drag");
    rect = el.getBoundingClientRect();
    onPoint(e, rect);
  });
  el.addEventListener("pointermove", (e) => el.hasPointerCapture(e.pointerId) && onPoint(e, rect));
  const up = () => el.classList.remove("drag");
  el.addEventListener("pointerup", up);
  el.addEventListener("pointercancel", up);
}

/** Arrow keys step a slider; Shift moves ten steps. */
function arrowStep(el: HTMLElement, step: number, move: (delta: number) => void) {
  el.addEventListener("keydown", (e) => {
    const d = e.key === "ArrowRight" || e.key === "ArrowUp" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowDown" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    move(d * step * (e.shiftKey ? 10 : 1));
  });
}

// ---------------------------------------------------------------- switch (38 x 22)
export interface Switch { el: HTMLButtonElement; set(on: boolean): void; get(): boolean }
export function toggle(label: string, onChange: (on: boolean) => void): Switch {
  let on = false;
  const el = h("button", { class: "switch", role: "switch", "aria-checked": "false", "aria-label": label, type: "button" }, h("span", { class: "knob" }));
  const set = (v: boolean) => {
    on = v;
    el.setAttribute("aria-checked", String(v));
  };
  el.addEventListener("click", () => {
    set(!on);
    onChange(on);
  });
  return { el, set, get: () => on };
}

// ---------------------------------------------------------------- big slider (Control Center style)
export interface BigSlider { el: HTMLElement; set(v: number, animate?: boolean): void; color(c: string, dark?: boolean): void }
export function bigSlider(opts: { label: string; glyph: string; min: number; max: number; step?: number; onInput: (v: number) => void }): BigSlider {
  const { min, max } = opts;
  const step = opts.step ?? 1;
  let value = min;
  let cancel = () => {};
  const fill = h("div", { class: "bs-fill" }, icon(opts.glyph, 22));
  const el = h("div", { class: "bigslider", role: "slider", tabindex: "0", "aria-label": opts.label, "aria-valuemin": min, "aria-valuemax": max, "aria-valuenow": min }, fill);
  const paint = (v: number) => {
    fill.style.setProperty("--p", String((v - min) / (max - min)));
  };
  const set = (v: number, animate = false) => {
    v = clamp(v, min, max);
    el.setAttribute("aria-valuenow", String(Math.round(v)));
    cancel();
    if (animate) {
      const from = value;
      cancel = tween(280, (t) => paint(from + (v - from) * t));
    } else paint(v);
    value = v;
  };
  const fromX = (x: number, r: DOMRect) => {
    const k = r.height; // fill never shrinks below the glyph
    const p = clamp((x - r.left - k) / (r.width - k), 0, 1); // the fill edge lands under the pointer
    return clamp(Math.round((min + p * (max - min)) / step) * step, min, max);
  };
  const commit = (v: number) => {
    set(v);
    opts.onInput(v);
  };
  dragTrack(el, (e, r) => commit(fromX(e.clientX, r)));
  arrowStep(el, step, (d) => commit(clamp(value + d, min, max)));
  return { el, set, color: (c, dark = false) => { el.style.setProperty("--tint", c); fill.classList.toggle("on-dark", dark); } };
}

// ---------------------------------------------------------------- vertical slider (Home style: tap toggles, drag dims)
export interface VSlider { el: HTMLElement; set(v: number, on: boolean): void; color(c: string): void }
export function verticalSlider(opts: { label: string; min: number; max: number; step: number; text: (v: number) => string; onInput: (v: number) => void; onToggle: () => void }): VSlider {
  const { min, max } = opts;
  let value = min;
  const glyphOn = icon("bulb", 30, 1.3);
  const glyphOff = icon("bulbslash", 30, 1.3);
  const el = h("div", { class: "vslider", role: "slider", tabindex: "0", "aria-orientation": "vertical", "aria-label": opts.label, "aria-valuemin": min, "aria-valuemax": max, "aria-valuenow": min },
    h("div", { class: "vs-fill" }), h("div", { class: "vs-glyph" }, glyphOn, glyphOff));
  const set = (v: number, on: boolean) => {
    value = clamp(v, min, max);
    el.classList.toggle("off", !on);
    glyphOn.hidden = !on;
    glyphOff.hidden = on;
    el.setAttribute("aria-valuenow", String(Math.round(value)));
    el.setAttribute("aria-valuetext", on ? opts.text(Math.round(value)) : "Off");
    el.style.setProperty("--p", String((value - min) / (max - min)));
  };
  // Past either end the slider stretches a little and resists, instead of stopping dead.
  const stretch = (over: number) => {
    const hgt = el.clientHeight;
    const r = (Math.abs(over) * hgt * 0.12) / (hgt + 0.12 * Math.abs(over));
    el.style.transformOrigin = over > 0 ? "50% 100%" : "50% 0%";
    el.style.transform = over ? `scaleY(${1 + r / hgt})` : "";
  };
  // The drag is relative to where it started, so touching the slider never makes the light jump.
  let start: { y: number; v: number; moved: boolean } | null = null;
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    el.setPointerCapture(e.pointerId);
    start = { y: e.clientY, v: value, moved: false };
  });
  el.addEventListener("pointermove", (e) => {
    if (!start) return;
    const dy = start.y - e.clientY;
    if (!start.moved && Math.abs(dy) < 4) return;
    start.moved = true;
    el.classList.add("drag");
    const raw = start.v + (dy / el.clientHeight) * (max - min);
    const v = clamp(Math.round(raw), min, max);
    stretch(((raw - v) / (max - min)) * el.clientHeight);
    opts.onInput(v);
  });
  el.addEventListener("pointerup", () => {
    if (start && !start.moved) opts.onToggle();
    start = null;
    el.classList.remove("drag");
    el.style.transform = "";
  });
  const drop = () => { start = null; el.classList.remove("drag"); el.style.transform = ""; };
  el.addEventListener("pointercancel", drop);
  el.addEventListener("lostpointercapture", drop);
  el.addEventListener("keydown", (e) => {
    if (e.key === " " || e.key === "Enter") { e.preventDefault(); opts.onToggle(); return; }
    const d = e.key === "ArrowUp" || e.key === "ArrowRight" ? 1 : e.key === "ArrowDown" || e.key === "ArrowLeft" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    opts.onInput(clamp(value + d * opts.step * (e.shiftKey ? 5 : 1), min, max));
  });
  set(min, false);
  return { el, set, color: (c) => el.style.setProperty("--tint", c) };
}

// ---------------------------------------------------------------- thin slider with round thumb
export interface ThinSlider { el: HTMLElement; set(v: number, animate?: boolean): void }
export function thinSlider(opts: { label: string; min: number; max: number; step?: number; gradient?: string; onInput: (v: number) => void }): ThinSlider {
  const { min, max } = opts;
  const step = opts.step ?? 1;
  let value = min;
  let cancel = () => {};
  const thumb = h("div", { class: "ts-thumb" });
  const track = h("div", { class: "ts-track" + (opts.gradient ? " data" : "") });
  if (opts.gradient) track.style.background = opts.gradient;
  const el = h("div", { class: "thinslider", role: "slider", tabindex: "0", "aria-label": opts.label, "aria-valuemin": min, "aria-valuemax": max, "aria-valuenow": min }, track, thumb);
  const paint = (v: number) => el.style.setProperty("--p", String((v - min) / (max - min)));
  const set = (v: number, animate = false) => {
    v = clamp(v, min, max);
    el.setAttribute("aria-valuenow", String(Math.round(v)));
    cancel();
    if (animate) {
      const from = value;
      cancel = tween(280, (t) => paint(from + (v - from) * t));
    } else paint(v);
    value = v;
  };
  const fromX = (x: number, r: DOMRect) => {
    const p = clamp((x - r.left - THUMB / 2) / (r.width - THUMB), 0, 1);
    return clamp(Math.round((min + p * (max - min)) / step) * step, min, max);
  };
  const commit = (v: number) => {
    set(v);
    opts.onInput(v);
  };
  dragTrack(el, (e, r) => commit(fromX(e.clientX, r)));
  arrowStep(el, step, (d) => commit(clamp(value + d, min, max)));
  return { el, set };
}

// ---------------------------------------------------------------- segmented control
export interface Segmented { el: HTMLElement; set(i: number): void }
export function segmented(labels: string[], aria: string, onChange: (i: number) => void): Segmented {
  let cur = 0;
  const ind = h("div", { class: "seg-ind" });
  const btns = labels.map((l, i) =>
    h("button", { type: "button", role: "radio", "aria-checked": "false", text: l, onClick: () => { set(i); onChange(i); } }),
  );
  const el = h("div", { class: "segmented", role: "radiogroup", "aria-label": aria }, ind, ...btns);
  el.style.setProperty("--n", String(labels.length));
  const set = (i: number) => {
    cur = i;
    el.style.setProperty("--i", String(i));
    btns.forEach((b, j) => b.setAttribute("aria-checked", String(j === cur)));
  };
  set(0);
  return { el, set };
}

// ---------------------------------------------------------------- colour picker (saturation/value + hue + hex)
export interface Picker { el: HTMLElement; side: HTMLElement; set(hex: string): void }
export function colorPicker(onInput: (hex: string) => void): Picker {
  let hsv: [number, number, number] = [30, 0.5, 1];
  const svThumb = h("div", { class: "sv-thumb" });
  const sv = h("div", { class: "sv", role: "application", "aria-label": "Saturation and brightness", tabindex: "0" }, svThumb);
  const hueThumb = h("div", { class: "ts-thumb" });
  const hue = h("div", { class: "hue", role: "slider", tabindex: "0", "aria-label": "Hue", "aria-valuemin": 0, "aria-valuemax": 360, "aria-valuenow": 0 }, h("div", { class: "ts-track data hue-track" }), hueThumb);
  const hex = h("input", { class: "field hex", type: "text", spellcheck: "false", maxlength: "7", "aria-label": "Hex colour", autocomplete: "off" }) as HTMLInputElement;
  const swatch = h("span", { class: "swatch" });
  const paint = () => {
    const c = hsvToRgb(...hsv);
    sv.style.setProperty("--hue", css(hsvToRgb(hsv[0], 1, 1)));
    sv.style.setProperty("--x", String(hsv[1]));
    sv.style.setProperty("--y", String(1 - hsv[2]));
    svThumb.style.background = css(c);
    hue.style.setProperty("--p", String(hsv[0] / 360));
    hue.setAttribute("aria-valuenow", String(Math.round(hsv[0])));
    hueThumb.style.background = css(hsvToRgb(hsv[0], 1, 1));
    swatch.style.background = css(c);
    if (document.activeElement !== hex) hex.value = rgbToHex(c);
    return rgbToHex(c);
  };
  const emit = () => onInput(paint());
  dragTrack(sv, (e, r) => {
    hsv = [hsv[0], clamp((e.clientX - r.left) / r.width, 0, 1), 1 - clamp((e.clientY - r.top) / r.height, 0, 1)];
    emit();
  });
  dragTrack(hue, (e, r) => {
    hsv = [clamp((e.clientX - r.left - THUMB / 2) / (r.width - THUMB), 0, 1) * 360, hsv[1], hsv[2]];
    emit();
  });
  hue.addEventListener("keydown", (e) => {
    const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    hsv = [(hsv[0] + d * (e.shiftKey ? 15 : 3) + 360) % 360, hsv[1], hsv[2]];
    emit();
  });
  sv.addEventListener("keydown", (e) => {
    const k = e.key;
    if (!k.startsWith("Arrow")) return;
    e.preventDefault();
    const st = e.shiftKey ? 0.1 : 0.02;
    hsv = [hsv[0], clamp(hsv[1] + (k === "ArrowRight" ? st : k === "ArrowLeft" ? -st : 0), 0, 1), clamp(hsv[2] + (k === "ArrowUp" ? st : k === "ArrowDown" ? -st : 0), 0, 1)];
    emit();
  });
  const fromHex = () => {
    const t = hex.value.trim().replace(/^#?/, "#");
    if (/^#[0-9a-fA-F]{6}$/.test(t)) {
      hsv = rgbToHsv(hexToRgb(t));
      emit();
      return true;
    }
    return false;
  };
  hex.addEventListener("focus", () => hex.select());
  hex.addEventListener("input", fromHex);
  hex.addEventListener("blur", () => { if (!fromHex()) paint(); });
  hex.addEventListener("keydown", (e) => e.key === "Enter" && hex.blur());
  const side = h("div", { class: "side" });
  side.append(h("label", { class: "hexrow" }, swatch, hex));
  const el = h("div", { class: "picker" }, sv, side, hue);
  paint();
  return {
    el,
    side,
    set: (x) => {
      const c = hexToRgb(x);
      const n = rgbToHsv(c);
      // keep the hue when the colour is grey, so the hue slider does not jump to red
      hsv = n[1] < 0.02 || n[2] < 0.02 ? [hsv[0], n[1], n[2]] : n;
      paint();
    },
  };
}
