// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The page's colors from the theme (docs/UI-GUIDELINES.md §2): the accent (its cursor color, or
// its text color for a grey cursor that hides; also the focus ring and a primary key's fill under
// the background's color) and links (its link color), each lightened on a dark background or
// darkened on a light one until it reads (4.5:1).

/** "#rgb" or "#rrggbb" as [r, g, b], each 0…1; null for anything else. */
function rgb(hex) {
  const m = /^#([\da-f]{3}|[\da-f]{6})$/i.exec(hex ?? "");
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
}

/** WCAG relative luminance of "#rgb" or "#rrggbb"; null for anything else. */
function luminance(hex) {
  const c = rgb(hex);
  if (!c) return null;
  const [r, g, b] = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** The contrast ratio of two colors (1 … 21); null when one is not a hex color. */
export function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)];
  return x === null || y === null ? null : (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** "#rgb" or "#rrggbb" as [h 0…360, s 0…1, l 0…1]; null for anything else. */
function hsl(hex) {
  const c = rgb(hex);
  if (!c) return null;
  const [r, g, b] = c;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), l = (max + min) / 2, d = max - min;
  if (!d) return [0, 0, l];
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, d / (1 - Math.abs(2 * l - 1)), l];
}
const toHex = ([h, s, l]) => {
  const k = (n) => (n + h / 30) % 12, a = s * Math.min(l, 1 - l);
  return "#" + [0, 8, 4].map((n) => Math.round((l - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1))) * 255).toString(16).padStart(2, "0")).join("");
};

/** A color that reads on bg (4.5:1, WCAG's for text): the theme's own while it does, else its
 *  hue and saturation lighter on a dark background, darker on a light one, by 2% steps; not
 *  measurable (not a hex color): as the theme says. Text in bg's color reads on it as well. */
export function readable(bg, color) {
  const c = hsl(color);
  if (!c || (contrast(bg, color) ?? 21) >= 4.5) return color;
  const step = contrast(bg, "#ffffff") >= contrast(bg, "#000000") ? 0.02 : -0.02;
  let out = color;
  for (let i = 1; i <= 50 && contrast(bg, out) < 4.5; i++) out = toHex([c[0], c[1], Math.min(1, Math.max(0, c[2] + step * i))]);
  return out;
}

/** A theme's accent: its cursor color, made to read on bg; a grey cursor that does not read (an
 *  inverted one, white on a white theme) has no hue to keep, so the text color instead. */
export function accentColor(bg, cursor, fg) {
  const grey = (contrast(bg, cursor) ?? 21) < 4.5 && hsl(cursor)?.[1] < 0.1;
  return readable(bg, grey ? fg : cursor);
}

/** A theme's link color (its `link`, else its ANSI 12, as View's), made to read on its bg; undefined without one. */
export const themeLink = (theme) => { const l = theme.link || theme.ansi?.[12]; return l && readable(theme.bg, l); };

/** The page's colors from a theme: the terminal's, the accent (tokens.css: the focus ring, a
 *  primary key's fill), links, the selection and the browser's bar. */
export function pageColors(theme) {
  const r = document.documentElement.style;
  r.setProperty("--t-bg", theme.bg); r.setProperty("--t-fg", theme.fg);
  r.setProperty("--accent", accentColor(theme.bg, theme.cursor || theme.fg, theme.fg));
  const link = themeLink(theme);                       // as View colors links (ui/src/theme.ts); else tokens.css's
  if (link) r.setProperty("--link", link); else r.removeProperty("--link");
  document.documentElement.classList.toggle("light", theme.dark === false);   // tokens.css: lighter shadows, darker folder hues
  if (theme.sel) r.setProperty("--sel", theme.sel);
  if (theme.selfg) r.setProperty("--selfg", theme.selfg);
  document.querySelector('meta[name="theme-color"]').content = theme.bg;
}
