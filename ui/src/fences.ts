// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A rendered document's fences, shared by the viewer (markdown.ts) and a report's pane (report-doc.ts): no sign-in,
// no file read here, so the page's renderers (web-widgets.ts) carry it without the panel's API.

import { highlightInto } from "./highlight";
import { drawFence } from "./mermaid";

/** A rendered document's fenced code highlighted and its ```mermaid fences drawn as diagrams: the
 *  viewer's and a report's (web-widgets.ts, terminum-reports.md AC-268). */
export function drawFences(el: HTMLElement) {
  for (const code of el.querySelectorAll<HTMLElement>("pre > code[class*='language-']")) {
    const lang = [...code.classList].find((c) => c.startsWith("language-"))!.slice(9);
    code.parentElement!.dataset.lang = lang;
    if (/^(mermaid|mermaidjs|mmd)$/i.test(lang)) void drawFence(code.parentElement!, code.textContent ?? "");
    else void highlightInto(code, code.textContent ?? "", lang);
  }
}
