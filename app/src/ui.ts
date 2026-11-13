import { h } from "./dom";

type K = Node | string | null | false | undefined;

export function row(label: K, right?: K, sub?: string): HTMLElement {
  return h("div", { class: "row" }, h("div", { class: "label" }, label, sub ? h("small", { text: sub }) : null), right ? h("div", { class: "right" }, right) : null);
}

/** Full-width row (slider etc.) with a title line and a value on the right. */
export function stack(label: string, value: HTMLElement | null, control: Node): HTMLElement {
  return h("div", { class: "row stack" }, h("div", { class: "line" }, h("span", { class: "label", text: label }), value), control);
}

export const value = () => h("span", { class: "value" });

/** "250 lm": a number and its unit, joined so they never wrap apart. */
export const unit = (v: number, u: string) => `${v}\u00a0${u}`;
