// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An HTML document shown in a frame without scripts: the viewer's HTML files (docview.ts) and a report's HTML body
// (report-doc.ts, terminum-reports.md AC-268). Which local resources it may read is its caller's word.

import { hasScheme } from "./paths";

/** Make an HTML document safe and self-contained: no scripts or frames, a local resource fetched
 *  with the sign-in once it is shown from the path `local(url)` gives (`data-raw`: the viewer's
 *  hydrateRaw, a report's pane's own), dropped when it gives "", no address with a secret in the
 *  document; the panel's colors at `colorsAt` as a fallback for unstyled pages. */
export function safeHtml(text: string, local: (url: string) => string, colorsAt: Element = document.documentElement): string {
  const doc = new DOMParser().parseFromString(text, "text/html");
  doc.querySelectorAll("script, iframe, frame, object, embed, base, meta").forEach((el) => el.remove());
  for (const el of doc.querySelectorAll("*")) {
    // handlers, and the file's own data-raw: only the paths below are fetched with the sign-in
    for (const attr of [...el.attributes]) if (/^on|^data-raw$/i.test(attr.name)) el.removeAttribute(attr.name);
  }
  const isLocal = (url: string | null): url is string => !!url && !hasScheme(url) && !/^(#|\/\/)/.test(url);
  for (const el of doc.querySelectorAll<HTMLElement>("img[src], source[src], video[src], audio[src], input[src]")) {
    const src = el.getAttribute("src");
    if (!isLocal(src)) continue;
    const file = local(src);
    if (file) el.dataset.raw = file;
    el.removeAttribute("src");
  }
  for (const el of doc.querySelectorAll<HTMLElement>('link[rel~="stylesheet"][href]')) {
    const href = el.getAttribute("href"), file = isLocal(href) ? local(href) : "";
    if (file) { el.dataset.raw = file; el.removeAttribute("href"); } else el.remove();
  }
  const cs = getComputedStyle(colorsAt);
  const base = doc.createElement("style");
  base.textContent = `html{color-scheme:${cs.colorScheme || "normal"}}body{margin:0;padding:20px 24px;` +
    `font:15px/1.6 -apple-system,BlinkMacSystemFont,sans-serif;color:${cs.getPropertyValue("--fg")};` +
    `background:${cs.getPropertyValue("--bg")}}a{color:${cs.getPropertyValue("--link")}}img{max-width:100%}`;
  doc.head.prepend(base); // first, so the page's own styles win
  // the document's own policy: nothing may load from outside fbd, whatever the parent passes down
  const csp = doc.createElement("meta");
  csp.httpEquiv = "Content-Security-Policy";
  csp.content = "default-src 'none'; img-src 'self' data: blob:; media-src 'self' blob:; style-src 'self' 'unsafe-inline'; font-src 'self' data:";
  const referrer = Object.assign(doc.createElement("meta"), { name: "referrer", content: "no-referrer" });
  doc.head.prepend(csp, referrer);
  return "<!doctype html>" + doc.documentElement.outerHTML;
}
