// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The renderers of the web app's output widgets (AC-56), served by fbd as /renderers.js and loaded
// by the web page through its /fb proxy on first need: the panel's own Markdown rules (raw HTML
// off, only safe link schemes) and highlighter, and Mermaid (mermaid.ts), loaded only for a diagram.

import { ownFile, renderMd } from "./markdown-core";
import { highlightInto } from "./highlight";
import "./doc.css";   // dist/renderers.css: a rendered document's look, which a report's pane loads

/** Markdown → HTML, as the panel renders a document; images show as their alt text (the page
 *  loads no image a terminal's text names, local or remote). Links are the page's to handle. */
export const markdownHtml = (text: string): string => renderMd(text, { dir: "/", imagePath: () => "" });
/** A task's Markdown (terminum-kanban.md AC-109): the same rules, and its own files' pictures. */
/** said: a comment's, its label lines set apart (said.ts, AC-149). */
export const taskMarkdownHtml = (text: string, said = false): string => renderMd(text, { dir: "/", imagePath: ownFile, said });

/** A report's body as the viewer shows a document (report-doc.ts, terminum-reports.md AC-268). */
export { reportInto } from "./report-doc";

/** Highlights `code` into `into` (tok-* spans); false when the language is not known. `lang`
 *  is a fence's word ("py") or a file's name ("tool.py"). */
export const highlight = (into: HTMLElement, code: string, lang: string) => highlightInto(into, code, lang);

/** A Mermaid diagram as its SVG markup, or as an image at its size (mermaid.ts), strict, as the
 *  panel draws one. */
export { diagramImage, mermaidSvg } from "./mermaid";
