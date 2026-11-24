// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A report's body as the viewer shows a document (terminum-reports.md AC-268): Markdown by the panel's rules, its code
// highlighted and its ```mermaid fences drawn (fences.ts), or, when it is an HTML document, in a frame without scripts
// (html-safe.ts). A report has no folder: its only pictures are its own files (/api/task-files/<id>), which the page
// fetches with its sign-in (`data-raw`); any other local path is never read.

import { ownFile, renderMd } from "./markdown-core";
import { drawFences } from "./fences";
import { safeHtml } from "./html-safe";

export type DocFormat = "markdown" | "html";

/** A body is an HTML document when it starts as one; else Markdown. */
export const formatOf = (text: string): DocFormat => (/^\s*<(!doctype\s+html|html)[\s>]/i.test(text) ? "html" : "markdown");

/** Renders report body `text` into `box`, whose panel tokens (--fg, --bg, --link) give an HTML body its fallback
 *  colors; says which format it took. */
export function reportInto(box: HTMLElement, text: string): DocFormat {
  if (formatOf(text) === "html") {
    const frame = document.createElement("iframe");
    frame.className = "html-doc";
    frame.setAttribute("sandbox", "allow-same-origin");   // no scripts; same origin so the page routes its links
    frame.srcdoc = safeHtml(text, ownFile, box);
    box.replaceChildren(frame);
    return "html";
  }
  box.innerHTML = renderMd(text, { dir: "/", imagePath: ownFile });
  drawFences(box);
  return "markdown";
}
