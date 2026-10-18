// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The focus ring's color (docs/UI-GUIDELINES.md §2.6): the theme's accent (its cursor color),
// unless that is too near the background to see (under 3:1), then the theme's text color.

/** WCAG relative luminance of "#rgb" or "#rrggbb"; null for anything else. */
function luminance(hex) {
  const m = /^#([\da-f]{3}|[\da-f]{6})$/i.exec(hex ?? "");
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** The contrast ratio of two colors (1 … 21); null when one is not a hex color. */
export function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)];
  return x === null || y === null ? null : (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** The ring for a theme: its accent when it stands out from bg (or cannot be measured), else fg. */
export const ringColor = (bg, accent, fg) => ((contrast(bg, accent) ?? 3) >= 3 ? accent : fg);
