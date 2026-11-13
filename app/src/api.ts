import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface State {
  connected: boolean;
  model: string;
  hasBack: boolean;
  frontOn: boolean;
  brightness: number;
  temperature: number;
  backOn: boolean;
  backBrightness: number;
  zones: string[];
  error: string | null;
}

export interface Preset {
  id: string;
  name: string;
  builtin: boolean;
  icon: string;
  frontOn: boolean;
  brightness: number;
  temperature: number;
  backOn: boolean;
  backBrightness: number;
  zones: string[];
}

export interface AdaptiveSettings {
  enabled: boolean;
  mode: "sun" | "fixed";
  lat: number | null;
  lon: number | null;
  sunriseMin: number;
  sunsetMin: number;
  dayTemp: number;
  nightTemp: number;
  dayBrightness: number;
  nightBrightness: number;
  rampMin: number;
  includeBack: boolean;
  backDay: number;
  backNight: number;
  turnOn: boolean;
  pauseMode: "nextTransition" | "oneHour" | "tomorrow";
}

export interface AdaptiveView {
  settings: AdaptiveSettings;
  status: string;
  next: string;
  pausedText: string | null;
  targetTemp: number;
  targetBrightness: number;
  targetBack: number;
  nowMin: number;
  sunrise: number;
  sunset: number;
  fallback: boolean;
  curve: [number, number, number][];
}

let onError: (m: string) => void = () => {};
export const setErrorHandler = (f: (m: string) => void) => (onError = f);

export const msg = (e: unknown): string => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

/** Fire a command; failures are shown, never swallowed. */
export async function call<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    onError(msg(e));
    return undefined;
  }
}

/** Last value wins. At most one write in flight, at least `gap` ms between write starts. */
export function throttled<T>(fn: (v: T) => Promise<unknown>, gap = 60): (v: T) => void {
  let pending: { v: T } | null = null;
  let busy = false;
  let last = 0;
  const pump = async () => {
    if (busy) return;
    busy = true;
    while (pending) {
      const wait = last + gap - performance.now();
      if (wait > 0) await new Promise((r) => setTimeout(r, wait));
      const { v } = pending;
      pending = null;
      last = performance.now();
      await fn(v);
    }
    busy = false;
  };
  return (v: T) => {
    pending = { v };
    void pump();
  };
}

export const on = <T>(name: string, f: (p: T) => void) => listen<T>(name, (e) => f(e.payload));
