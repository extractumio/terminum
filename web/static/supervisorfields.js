// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { asTyped } from "./astyped.js";
import { el } from "./listparts.js";
import { fillSelect } from "./kanbanutil.js";

export const hint = (text) => el("p", "colempty", text);
export const grid = (...nodes) => part("div", "fieldgrid", ...nodes);
export const stack = (...nodes) => part("div", "sv-stack", ...nodes);
export const actions = (...nodes) => part("div", "sv-actions", ...nodes);

export function part(tag, cls, ...nodes) {
  const node = el(tag, cls);
  node.append(...nodes);
  return node;
}

export function section(title, ...nodes) {
  const summary = el("summary", "sv-touch", title);
  const box = part("details", "more sv-section", summary, stack(...nodes));
  box.dataset.section = title;
  return box;
}

export function select(options, value) {
  const node = el("select", "field");
  fillSelect(node, options, value);
  return node;
}

export function check(checked) {
  const node = el("input");
  node.type = "checkbox";
  node.checked = checked;
  return node;
}

export function text(value, type = "text") {
  const node = el(type === "area" ? "textarea" : "input", "field");
  if (type === "area") node.rows = 3;
  else node.type = type;
  node.value = value ?? "";
  return node;
}

export function literal(node) {
  for (const attr of ["autocomplete", "autocapitalize", "autocorrect"]) node.setAttribute(attr, "off");
  node.spellcheck = false;
  asTyped(node);
  return node;
}

export function number(value, { min = 0, max, step = "any", placeholder } = {}) {
  const node = text(value, "number");
  node.min = min;
  if (max !== undefined) node.max = max;
  node.step = step;
  node.inputMode = step === 1 ? "numeric" : "decimal";
  if (placeholder !== undefined) node.placeholder = placeholder;
  return node;
}

export function row(label, field) {
  field.setAttribute("aria-label", label);
  return part("label", "fieldrow", label, field);
}
export const tick = (label, field) => part("label", "sv-tick sv-touch", field, label);
export const timeOf = (field) => field.value ? new Date(field.value).getTime() / 1000 : null;

export function dateValue(seconds, type) {
  if (seconds == null) return "";
  const date = new Date(seconds * 1000);
  if (!Number.isFinite(date.getTime())) return "";
  if (type === "date") return date.toISOString().slice(0, 10);
  return new Date(date - date.getTimezoneOffset() * 60000).toISOString().slice(0, 19);
}

let errors = 0;
export function fieldError(field, label, words) {
  for (let group = field.closest("details"); group; group = group.parentElement.closest("details")) group.open = true;
  const error = new Error(`${label}: ${words}`);
  error.field = field;
  if (!field.hasAttribute("aria-invalid")) {
    const message = el("p", "sv-field-error", error.message);
    message.id = `supervisor-field-error-${++errors}`;
    message.setAttribute("role", "alert");
    field.closest(".fieldrow").after(message);
    field.setAttribute("aria-invalid", "true");
    field.setAttribute("aria-describedby", message.id);
    field.addEventListener("input", () => {
      message.remove();
      field.removeAttribute("aria-invalid");
      field.removeAttribute("aria-describedby");
    }, { once: true });
  }
  field.focus();
  throw error;
}

export function editor() {
  const reads = [];
  function field(label, node, apply, read) {
    const checked = node.type === "checkbox";
    const value = () => checked ? node.checked : node.value;
    const initial = value();
    reads.push((draft) => {
      if (!node.validity.valid) fieldError(node, label, node.validationMessage);
      if (value() !== initial) apply(read ? read(node) : value(), draft);
    });
    return checked ? tick(label, node) : row(label, node);
  }
  return { field, raw: (read) => reads.push(read), read: (draft) => { for (const read of reads) read(draft); return draft; } };
}
