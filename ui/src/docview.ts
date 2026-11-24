// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Rendered documents (Markdown and HTML; JSON, YAML, CSV and TSV: dataview.ts) and the links inside them. A link never navigates
// the panel: anchors scroll, web links go to the default browser (in the web app: the reader's browser), files open as tabs
// (`guide.md#install` opens guide.md and scrolls to "install").

import { api, apiOrToast, basename, dirname, decodePath, hasScheme, hydrateRaw, PROXIED, resolvePath, SERVER_APPS, toast, type FileView } from "./api";
import { safeHtml } from "./html-safe";
import { isMarkdown, renderMarkdown } from "./markdown";
import { dataKind } from "./dataparse";
import { renderData } from "./dataview";

export const isHtml = (path: string) => /\.x?html?$/i.test(path);
export const isRenderable = (path: string) => isMarkdown(path) || isHtml(path) || dataKind(path) !== null;

export interface LinkHost {
  host: string | null;
  current(): boolean;
  open(path: string, anchor?: string): void;
  prefetched(path: string, file: FileView): void; // the link check already read the file
}

/** Scroll `scope` to the element with id (or name) `anchor`. */
export function scrollToAnchor(scope: Document | HTMLElement, anchor: string, smooth = true) {
  const id = CSS.escape(anchor);
  scope.querySelector(`[id="${id}"], [name="${id}"]`)?.scrollIntoView({ behavior: smooth ? "smooth" : "instant", block: "start" });
}

/** Follow `href` found in the document at `from`; `scope` is where in-page anchors live. */
export async function followLink(href: string, from: string, scope: Document | HTMLElement, host: LinkHost) {
  if (href.startsWith("#")) return scrollToAnchor(scope, decodePath(href.slice(1)));
  if (/^(https?:|mailto:)/i.test(href)) {
    // in the web app (AC-52) the reader is on another device: open it there, not on the Mac
    if (PROXIED || !SERVER_APPS) return void window.open(href, "_blank", "noopener,noreferrer");
    return void apiOrToast("POST", "/api/os/open", { body: { url: href } });
  }
  if (hasScheme(href) && !href.startsWith("file:")) return toast(`Not opened: ${href.split(":")[0]} links`);
  const [file, anchor] = href.replace(/^file:\/\//, "").split("#");
  let target: string;
  try { target = resolvePath(dirname(from), decodeURIComponent(file)); } catch { return toast(`Bad link: ${href}`); }
  try {
    const file = await api<FileView>("GET", "/api/file", { query: { path: target }, host: host.host });
    if (!host.current()) return;
    host.prefetched(target, file);
    host.open(target, anchor ? decodePath(anchor) : undefined);
  } catch (e: any) {
    toast(`${basename(target)}: ${e.message}`);
  }
}

/** Render `text` (the document at `path`) into `box`, wire its links, scroll to `anchor`. */
export function renderDoc(box: HTMLElement, text: string, path: string, host: LinkHost, anchor?: string) {
  const onClick = (scope: Document | HTMLElement) => (e: MouseEvent) => {
    const a = (e.target as Element).closest?.("a[href]");
    if (!a) return;
    e.preventDefault();
    void followLink(a.getAttribute("href")!, path, scope, host);
  };
  if (dataKind(path)) return renderData(box, text, path);
  if (isMarkdown(path)) {
    box.innerHTML = '<article class="md-body"></article>';
    const article = box.firstElementChild as HTMLElement;
    renderMarkdown(article, text, path);
    article.addEventListener("click", onClick(article));
    if (anchor) requestAnimationFrame(() => scrollToAnchor(article, anchor, false));
    return;
  }
  // HTML: a sandboxed frame without scripts; same origin only so we can route its links
  const frame = document.createElement("iframe");
  frame.className = "html-doc";
  frame.setAttribute("sandbox", "allow-same-origin");
  frame.srcdoc = prepareHtml(text, path);
  frame.addEventListener("load", () => {
    const doc = frame.contentDocument;
    if (!doc) return;
    hydrateRaw(doc);
    doc.addEventListener("click", onClick(doc));
    if (anchor) scrollToAnchor(doc, anchor, false); // opening at a section: jump, do not animate
  });
  box.replaceChildren(frame);
}

/** An HTML file made safe (html-safe.ts), its local resources read from its own folder. */
const prepareHtml = (text: string, path: string): string => safeHtml(text, (url) => resolvePath(dirname(path), decodePath(url.split(/[?#]/)[0])));
