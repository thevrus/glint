export type RGB = [number, number, number];

export function kelvinRgb(k: number): RGB {
  const t = k / 100;
  const c = (v: number) => Math.round(Math.min(255, Math.max(0, v)));
  const r = t <= 66 ? 255 : 329.698727446 * Math.pow(t - 60, -0.1332047592);
  const g = t <= 66 ? 99.4708025861 * Math.log(t) - 161.1195681661 : 288.1221695283 * Math.pow(t - 60, -0.0755148492);
  const b = t >= 66 ? 255 : t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447927307;
  return [c(r), c(g), c(b)];
}

/** CSS gradient across the light's colour temperature range. */
export const kelvinGradient = () => `linear-gradient(to right, ${[2700, 3400, 4100, 4800, 5500, 6500].map((k) => css(kelvinRgb(k))).join(", ")})`;

export const hexToRgb = (h: string): RGB => {
  const v = parseInt(h.replace("#", "").padEnd(6, "0").slice(0, 6), 16);
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255];
};
export const rgbToHex = (c: RGB) => "#" + c.map((x) => Math.round(x).toString(16).padStart(2, "0")).join("");
export const css = (c: RGB) => `rgb(${Math.round(c[0])} ${Math.round(c[1])} ${Math.round(c[2])})`;
export const mix = (a: RGB, b: RGB, t: number): RGB => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];

export function hsvToRgb(h: number, s: number, v: number): RGB {
  const f = (n: number) => {
    const k = (n + h / 60) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return [f(5) * 255, f(3) * 255, f(1) * 255];
}
export function rgbToHsv([r, g, b]: RGB): [number, number, number] {
  r /= 255; g /= 255; b /= 255;
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn;
  let h = 0;
  if (d) h = mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [(h * 60 + 360) % 360, mx ? d / mx : 0, mx];
}

export const prefersReducedMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
export const ease = (t: number) => 1 - Math.pow(1 - t, 3);

/** Animate a number tween; returns cancel. Instant under reduced motion. */
export function tween(ms: number, step: (t: number) => void, done?: () => void): () => void {
  if (prefersReducedMotion()) {
    step(1);
    done?.();
    return () => {};
  }
  const t0 = performance.now();
  let raf = 0;
  const tick = (now: number) => {
    const t = Math.min(1, (now - t0) / ms);
    step(ease(t));
    if (t < 1) raf = requestAnimationFrame(tick);
    else done?.();
  };
  raf = requestAnimationFrame(tick);
  return () => cancelAnimationFrame(raf);
}
