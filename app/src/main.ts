import "./style.css";
import { call, on, setErrorHandler, throttled, type AdaptiveView, type Preset, type State } from "./api";
import { bigSlider, colorPicker, segmented, thinSlider, toggle, verticalSlider } from "./controls";
import { createAdaptive } from "./adaptive-ui";
import { css, kelvinGradient, kelvinRgb, mix, hexToRgb, type RGB } from "./color";
import { h, icon } from "./dom";
import { createHero } from "./hero";
import { row, setPressed, stack, unit, value } from "./ui";

document.documentElement.classList.toggle("mac", /Mac/.test(navigator.userAgent));

let st: State | null = null;
let zone = 1;
let all = false;
let activeId: string | null = null;
const touched: Record<string, number> = {};
const touch = (k: string) => {
  touched[k] = performance.now();
  if (activeId) { activeId = null; paintPresets(); }
};
const fresh = (k: string) => performance.now() - (touched[k] ?? -1e9) > 2500;
const safe = <T,>(f: () => T, d: T): T => { try { return f(); } catch { return d; } };

// ---------------------------------------------------------------- notice
const noticeText = h("span", { class: "text" });
const notice = h("div", { class: "notice", role: "status", hidden: true }, noticeText, h("button", { class: "link", type: "button", text: "Dismiss", onClick: () => (notice.hidden = true) }));
let noticeTimer = 0;
const notify = (m: string) => {
  noticeText.textContent = m;
  notice.hidden = false;
  window.clearTimeout(noticeTimer);
  // a long message stays until dismissed: nine seconds is not enough to read it
  if (m.length <= 140) noticeTimer = window.setTimeout(() => (notice.hidden = true), 9000);
};
setErrorHandler(notify);

// ---------------------------------------------------------------- hero
const hero = createHero(
  (z) => { if (matching) return; zone = z; all = false; syncZone(); redraw(); selectTab(1); },
  () => selectTab(0),
);
const heroState = () => ({
  on: !!st?.frontOn, brightness: st?.brightness ?? 52, temperature: st?.temperature ?? 4400,
  hasBack: !!st?.hasBack, backOn: !!st?.backOn, backBrightness: st?.backBrightness ?? 50,
  zones: st?.zones ?? [], selected: matching ? -1 : all ? 0 : zone,
});
const redraw = (instant = false) => hero.update(heroState(), instant);

// ---------------------------------------------------------------- light tab
// One writer per front-light property, shared by the simple and advanced views: local state,
// every control that shows it, the picture, then the device.
function setFront(on: boolean) { touch("front"); if (st) st.frontOn = on; frontPower.set(on); redraw(); paintFills(); void call("set_front_power", { on }); }
function setBack(on: boolean) { touch("backOn"); if (st) st.backOn = on; backPower.set(on); redraw(); paintFills(); void call("set_back_power", { on }); }
function setBright(lm: number) { touch("bright"); wake("front"); if (st) st.brightness = lm; bright.set(lm); brightVal.textContent = unit(lm, "lm"); redraw(true); paintFills(); sendBright(lm); }
/** `tap`: a swatch was chosen, so the light wakes and the picture eases; a drag tracks exactly. */
function setTemp(k: number, tap = false) { touch("temp"); if (tap) wake("front"); if (st) st.temperature = k; temp.set(k); tempVal.textContent = unit(k, "K"); redraw(!tap); paintFills(); sendTemp(k); }
const frontPower = toggle("Front light", setFront);
// Dragging a brightness slider while that light is off turns it on, as Control Center does.
function wake(side: "front" | "back") {
  if (!st) return;
  if (side === "front" && !st.frontOn) setFront(true);
  if (side === "back" && !st.backOn) setBack(true);
}
const brightVal = value();
const sendBright = throttled((lm: number) => call("set_front_brightness", { lm }), 60);
const bright = bigSlider({ label: "Front brightness", glyph: "sun", min: 30, max: 400, onInput: setBright });
const tempVal = value();
const sendTemp = throttled((kelvin: number) => call("set_temperature", { kelvin }), 60);
const temp = thinSlider({ label: "Colour temperature", min: 2700, max: 6500, step: 50, gradient: kelvinGradient(), onInput: (k) => setTemp(k) });
const lightBody = h("div", { class: "panel-body" }, row("Front light", frontPower.el), stack("Brightness", brightVal, bright.el), stack("Temperature", tempVal, temp.el));

// ---------------------------------------------------------------- back tab
const backPower = toggle("Back light", setBack);
const backVal = value();
const sendBackB = throttled((pct: number) => call("set_back_brightness", { pct }), 60);
const backBright = bigSlider({
  label: "Back brightness", glyph: "bulb", min: 1, max: 100,
  onInput: (p) => { touch("backB"); wake("back"); if (st) st.backBrightness = p; backVal.textContent = `${p}%`; redraw(true); paintFills(); sendBackB(p); },
});
const sendZone = throttled((a: { zone: number; color: string }) => call("set_back_zone", a), 80);
const sendAll = throttled((color: string) => call("set_back_all", { color }), 150);
const picker = colorPicker((hex) => {
  if (!st) return;
  touch("zones");
  if (all) { st.zones = Array(7).fill(hex); sendAll(hex); }
  else { st.zones = st.zones.slice(); st.zones[zone - 1] = hex; sendZone({ zone, color: hex }); }
  redraw(true); paintFills();
});
const zoneLabel = h("span", { class: "zone-label" });
const allBtn = h("button", { type: "button", class: "btn small", "aria-pressed": "false", text: "All Zones", onClick: () => { all = !all; syncZone(); redraw(); } });
picker.side.prepend(zoneLabel);
picker.side.append(allBtn);
function syncZone() {
  zoneLabel.textContent = matching ? "Screen" : all ? "All zones" : `Zone ${zone}`;
  setPressed(allBtn, all);
  // while the screen drives the zones the dimmed picker holds its colour instead of flickering along
  if (!matching) picker.set(st?.zones[Math.max(0, all ? 0 : zone - 1)] ?? "#ffffff");
}
// Match Screen: the back light follows the display; the picker steps aside meanwhile.
let matching = false;
let matchBusy = false;
const matchBtn = h("button", { type: "button", class: "btn small", "aria-pressed": "false", title: "Back light follows the colours on your screen", text: "Match Screen", onClick: async () => {
  // the first run can wait behind the Screen Recording prompt: no second toggle meanwhile
  if (matchBusy) return;
  matchBusy = true;
  const on = !matching;
  // a unit Ok arrives as null; call() gives undefined only on failure (and shows the notice)
  const ok = (await call("set_screen_sync", { on })) !== undefined;
  matchBusy = false;
  if (ok || !on) setMatching(on);
  if (ok && on) wake("back");
} });
const pickerRow = h("div", { class: "row picker-row" }, picker.el);
function setMatching(on: boolean) {
  matching = on;
  if (on) delete touched.zones;
  setPressed(matchBtn, on);
  pickerRow.classList.toggle("synced", on);
  syncZone();
  redraw();
}
const frag = (...n: Node[]) => { const f = document.createDocumentFragment(); f.append(...n); return f; };
const noBack = h("p", { class: "none", text: "This light has no back light." });
const backBody = h("div", { class: "panel-body" },
  row("Back light", frag(matchBtn, backPower.el)),
  h("div", { class: "row combo" }, backBright.el, backVal),
  pickerRow,
);

// ---------------------------------------------------------------- auto tab + settings sheet
const adaptive = createAdaptive();

// ---------------------------------------------------------------- tabs
const bodies = [lightBody, backBody, adaptive.tab];
const panel = h("div", { class: "group panel" }, ...bodies, noBack);
let tabIdx = Math.min(2, Math.max(0, Number(safe(() => localStorage.getItem("tab"), "0")) || 0));
const tabs = segmented(["Front", "Back", "Auto"], "View", (i) => selectTab(i));
/** Show the current tab's body. Idempotent: render() calls it on every state event. */
function paintTab() {
  const noB = tabIdx === 1 && st !== null && !st.hasBack;
  bodies.forEach((b, j) => (b.hidden = j !== tabIdx || noB));
  noBack.hidden = !noB;
}
/** The user picked a tab. */
function selectTab(i: number) {
  tabIdx = i;
  tabs.set(i);
  safe(() => localStorage.setItem("tab", String(i)), undefined);
  paintTab();
}
tabs.set(tabIdx);
paintTab();

// ---------------------------------------------------------------- simple view
// One tall slider (tap for power, drag for brightness) and a row of colour temperatures.
const simpleBright = verticalSlider({
  label: "Front light", min: 30, max: 400, step: 10, text: (v) => unit(v, "lm"), onInput: setBright,
  onToggle: () => { if (st) setFront(!st.frontOn); },
});
const TEMPS = [6500, 5500, 4400, 3400, 2700];
const tempSwatches = TEMPS.map((k) => {
  const b = h("button", { type: "button", class: "sw", title: unit(k, "K"), "aria-label": `${k} kelvin`, "aria-pressed": "false", onClick: () => setTemp(k, true) });
  b.style.setProperty("--c", css(kelvinRgb(k)));
  return b;
});
const backDot = h("span", {});
const backSwatch = h("button", { type: "button", class: "sw back", title: "Back Light Colours", "aria-label": "Back light colours", onClick: () => { setView(false); selectTab(1); } }, backDot);
const simpleView = h("div", { class: "simple-view simple-only" }, simpleBright.el, h("div", { class: "swatches", role: "group", "aria-label": "Colour temperature" }, ...tempSwatches, backSwatch));
let simple = safe(() => localStorage.getItem("view"), null) !== "advanced";
function setView(s: boolean) {
  if (s !== simple) safe(() => localStorage.setItem("view", s ? "simple" : "advanced"), undefined);
  simple = s;
  content.classList.toggle("simple", s);
  paintSub();
}

// ---------------------------------------------------------------- presets
const strip = h("div", { class: "strip", role: "group", "aria-label": "Presets" });
let presets: Preset[] = [];
// a plain mouse wheel scrolls the strip sideways
strip.addEventListener("wheel", (e) => { if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) { e.preventDefault(); strip.scrollLeft += e.deltaY; } }, { passive: false });

const nameSheet = h("dialog", { class: "sheet" }) as HTMLDialogElement;
function askName(title: string, initial: string, ok: string): Promise<string | null> {
  return new Promise((resolve) => {
    const input = h("input", { class: "field wide", type: "text", maxlength: "32", value: initial, "aria-label": "Name", autocomplete: "off", spellcheck: "false" }) as HTMLInputElement;
    const cancel = h("button", { type: "button", class: "btn", text: "Cancel" });
    const save = h("button", { type: "submit", class: "btn primary", text: ok });
    const form = h("form", { method: "dialog" }, h("h3", { text: title }), input, h("div", { class: "actions" }, cancel, save));
    nameSheet.replaceChildren(form);
    let result: string | null = null;
    form.addEventListener("submit", () => { result = input.value.trim() || null; });
    cancel.addEventListener("click", () => nameSheet.close());
    nameSheet.addEventListener("close", () => resolve(result), { once: true });
    nameSheet.showModal();
    input.select();
  });
}

function applyLocal(p: Preset) {
  if (!st) return;
  Object.assign(st, { frontOn: p.frontOn, brightness: p.brightness, temperature: p.temperature });
  if (st.hasBack) Object.assign(st, { backOn: p.backOn, backBrightness: p.backBrightness, zones: p.zones.slice() });
  // paint first: stamping the keys as touched is what makes paintControls skip them
  paintControls(true);
  for (const k of ["front", "bright", "temp", "backOn", "backB", "zones"]) touched[k] = performance.now();
}

function paintPresets() {
  strip.replaceChildren();
  for (const p of presets) {
    const on = p.id === activeId;
    strip.append(h("button", { type: "button", class: "tile" + (on ? " active" : ""), "aria-pressed": String(on), title: p.name, onClick: () => {
      activeId = p.id; applyLocal(p); paintPresets(); void call("apply_preset", { id: p.id });
    } }, icon(p.icon, 20), h("span", { class: "name", text: p.name })));
  }
  strip.append(h("button", { type: "button", class: "tile add", "aria-label": "Save Current as Preset", title: "Save Current as Preset", onClick: async () => {
    const n = await askName("New Preset", "", "Save");
    if (n) { const p = await call<Preset>("save_preset", { name: n }); if (p) activeId = p.id; }
  } }, icon("plus", 20)));
  paintManage();
}

// ---------------------------------------------------------------- gear sheet
const manageList = h("div", { class: "group" });
function paintManage() {
  manageList.replaceChildren();
  const mine = presets.filter((p) => !p.builtin);
  if (!mine.length) manageList.append(h("div", { class: "row" }, h("span", { class: "label none-inline", text: "No saved presets yet." })));
  for (const p of mine) {
    manageList.append(h("div", { class: "row" }, h("span", { class: "label", text: p.name }),
      h("button", { type: "button", class: "btn small", text: "Rename", onClick: async () => { const n = await askName("Rename Preset", p.name, "Rename"); if (n) void call("rename_preset", { id: p.id, name: n }); } }),
      h("button", { type: "button", class: "btn small danger", "aria-label": `Delete ${p.name}`, onClick: () => void call("delete_preset", { id: p.id }) }, icon("trash", 16))));
  }
}
const gearSheet = h("dialog", { class: "sheet settings", tabindex: "-1" }) as HTMLDialogElement;
// Focus lands on the sheet itself, not its first button: a Mac sheet shows no focus ring until Tab is pressed.
const openSettings = () => { gearSheet.showModal(); gearSheet.focus(); };
gearSheet.append(
  h("div", { class: "sheet-head" }, h("h3", { text: "Settings" }), h("button", { type: "button", class: "btn primary", text: "Done", onClick: () => gearSheet.close() })),
  h("div", { class: "sheet-body" },
    h("h4", { text: "Adaptive" }), adaptive.settings,
    h("h4", { text: "Presets" }), manageList),
);
// close on a click that both starts and ends on the backdrop, so a text drag released outside does not dismiss
let downOnBackdrop = false;
gearSheet.addEventListener("pointerdown", (e) => { downOnBackdrop = e.target === gearSheet; });
gearSheet.addEventListener("click", (e) => { if (e.target === gearSheet && downOnBackdrop) gearSheet.close(); });
const gear = h("button", { type: "button", class: "gear adv-only", "aria-label": "Settings", title: "Settings", onClick: openSettings }, icon("more", 20));
const advBtn = h("button", { type: "button", class: "gear simple-only", "aria-label": "Advanced controls", title: "Advanced", onClick: () => { setView(false); doneBtn.focus(); } }, icon("slider", 20));
const doneBtn = h("button", { type: "button", class: "btn small adv-only", text: "Done", onClick: () => { setView(true); advBtn.focus(); } });

// ---------------------------------------------------------------- shell
const title = h("h1", { text: "Litra Beam" });
const sub = h("p", { class: "sub" });
const header = h("header", {}, h("div", { class: "titlerow" }, title, h("div", { class: "hbtns" }, advBtn, doneBtn, gear)), sub);
const content = h("main", { class: "content" }, header, notice, simpleView, h("div", { class: "hero-wrap" }, hero.el), tabs.el, panel, strip);
content.classList.toggle("simple", simple);

const retry = h("button", { type: "button", class: "btn primary", text: "Try Again" }) as HTMLButtonElement;
retry.addEventListener("click", () => {
  retry.disabled = true; retry.textContent = "Searching…";
  void call("retry_connect");
  setTimeout(() => { retry.disabled = false; retry.textContent = "Try Again"; }, 2500);
});
const emptyDetail = h("p", { class: "reason" });
const empty = h("main", { class: "empty", hidden: true },
  h("div", { class: "big" }, icon("bulbslash", 56, 1.2)),
  h("h2", { text: "No Light Found" }),
  h("p", { text: "Turn on your Litra Beam LX and make sure it is paired in your Bluetooth settings." }),
  emptyDetail, retry);

document.getElementById("app")!.append(content, empty, gearSheet, nameSheet);

// ---------------------------------------------------------------- painting
function avgZones(): RGB {
  const z = (st?.zones ?? []).map(hexToRgb);
  if (!z.length) return [200, 200, 200];
  return [0, 1, 2].map((i) => z.reduce((a, c) => a + c[i], 0) / z.length) as RGB;
}
function paintFills() {
  const grey: RGB = [142, 142, 147];
  const dark = (c: RGB) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] < 120;
  const avg = avgZones();
  const back = st?.backOn ? mix(avg, [255, 255, 255], 0.15) : grey;
  const front = css(st ? kelvinRgb(st.temperature) : grey);
  bright.color(st?.frontOn ? front : css(grey));
  backBright.color(css(back), dark(back));
  lightBody.classList.toggle("off", !st?.frontOn);
  if (st) {
    simpleBright.set(st.brightness, st.frontOn);
    simpleBright.color(front);
    tempSwatches.forEach((b, i) => b.setAttribute("aria-pressed", String(Math.abs(st!.temperature - TEMPS[i]) <= 100)));
    backSwatch.hidden = !st.hasBack;
    backDot.style.background = css(avg);
    if (simple) paintSub();
  }
  backBody.classList.toggle("off", !st?.backOn);
}
function paintControls(animate: boolean) {
  if (!st) return;
  if (fresh("front")) frontPower.set(st.frontOn);
  if (fresh("bright")) { bright.set(st.brightness, animate); brightVal.textContent = unit(st.brightness, "lm"); }
  if (fresh("temp")) { temp.set(st.temperature, animate); tempVal.textContent = unit(st.temperature, "K"); }
  if (fresh("backOn")) backPower.set(st.backOn);
  if (fresh("backB")) { backBright.set(st.backBrightness, animate); backVal.textContent = `${st.backBrightness}%`; }
  if (fresh("zones") && !matching) syncZone();
  paintFills();
  redraw();
}
let firstPaint = true;
function render(next: State) {
  const keep = st;
  st = { ...next };
  // never show a value the controls cannot represent
  const lim = (v: number, a: number, b: number) => Math.min(b, Math.max(a, Number.isFinite(v) ? v : a));
  st.brightness = lim(st.brightness, 30, 400);
  st.temperature = lim(st.temperature, 2700, 6500);
  st.backBrightness = lim(st.backBrightness, 1, 100);
  if (st.zones.length && st.zones.length !== 7) st.zones = Array.from({ length: 7 }, (_, i) => st!.zones[i] ?? "#ffffff");
  if (keep) {
    if (!fresh("front")) st.frontOn = keep.frontOn;
    if (!fresh("bright")) st.brightness = keep.brightness;
    if (!fresh("temp")) st.temperature = keep.temperature;
    if (!fresh("backOn")) st.backOn = keep.backOn;
    if (!fresh("backB")) st.backBrightness = keep.backBrightness;
    if (!fresh("zones") && !matching) st.zones = keep.zones;
  }
  content.hidden = !st.connected;
  empty.hidden = st.connected;
  emptyDetail.textContent = st.error ?? "";
  emptyDetail.hidden = !st.error;
  title.textContent = (st.model || "Litra Beam").replace(" (Bluetooth)", "");
  title.title = title.textContent;
  paintSub();
  paintTab();
  paintControls(!firstPaint);
  if (firstPaint) {
    firstPaint = false;
    requestAnimationFrame(() => requestAnimationFrame(() => void call("window_ready", { connected: st?.connected ?? false })));
  }
}

let av: AdaptiveView | null = null;
function paintSub() {
  if (simple && st) { sub.textContent = st.frontOn ? `${unit(st.brightness, "lm")} · ${unit(st.temperature, "K")}` : "Off"; return; }
  const link = st?.connected ? (st.model.includes("Bluetooth") ? "Bluetooth" : "USB") : "";
  const auto = !av?.settings.enabled ? "" : av.pausedText ? "Adaptive paused" : "Adaptive";
  sub.textContent = [link, auto].filter(Boolean).join(" · ");
}
function renderAdaptive(v: AdaptiveView) {
  av = v;
  adaptive.update(v);
  paintSub();
}

// ---------------------------------------------------------------- boot
// The backend injects the current state before any script runs: paint it now, with no IPC.
const boot = (window as { __BOOT__?: { state: State; presets: Preset[]; adaptive: AdaptiveView } }).__BOOT__;
if (boot) {
  presets = boot.presets;
  paintPresets();
  renderAdaptive(boot.adaptive);
  // At cold launch the light has not been found yet; wait for get_state rather than
  // flashing the empty screen. Reopening from the tray paints the live state at once.
  if (boot.state.connected) render(boot.state);
}
(async () => {
  await Promise.all([
    on<State>("state", render),
    on<AdaptiveView>("adaptive", renderAdaptive),
    on<Preset[]>("presets", (p) => { presets = p; paintPresets(); }),
    on<string>("notice", notify),
    on<boolean>("screen-sync", setMatching),
  ]);
  const [s0, p0, a0, sync] = await Promise.all([call<State>("get_state"), call<Preset[]>("list_presets"), call<AdaptiveView>("get_adaptive"), call<boolean>("get_screen_sync")]);
  setMatching(!!sync);
  presets = p0 ?? [];
  paintPresets();
  // also covers anything emitted between the injected snapshot and the listeners going live
  if (a0) renderAdaptive(a0);
  if (s0) render(s0);
})();

// ---------------------------------------------------------------- native window behaviour
// Accent controls go quiet while another window is key, as in any Mac app.
window.addEventListener("blur", () => document.documentElement.classList.add("inactive"));
window.addEventListener("focus", () => document.documentElement.classList.remove("inactive"));
// No web context menu (Reload, Inspect) outside text fields.
window.addEventListener("contextmenu", (e) => { if (!(e.target instanceof HTMLInputElement)) e.preventDefault(); });
// Command-comma opens Settings.
window.addEventListener("keydown", (e) => {
  if (e.metaKey && e.key === "," && !gearSheet.open && !nameSheet.open && st?.connected) { e.preventDefault(); openSettings(); }
});
