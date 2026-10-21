// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Mermaid, loaded only for a diagram: the terminal's output widgets (web-widgets.ts, AC-31) and
// the ```mermaid fences of a rendered document (markdown.ts, AC-54) draw with the same one.

let mermaidLib: Promise<typeof import("mermaid").default> | null = null;
let shownTheme = "";
let seq = 0;

/** A Mermaid diagram's SVG markup. Strict: no click handlers, no HTML labels (the SVG is shown
 *  as an image, where HTML inside it would not draw). Throws Mermaid's message on bad source. */
export async function mermaidSvg(source: string, dark: boolean): Promise<string> {
  mermaidLib ??= import("mermaid").then((m) => m.default);
  const mermaid = await mermaidLib;
  const theme = dark ? "dark" : "default";
  if (theme !== shownTheme) {
    // Mermaid's dark pie slices are near black on a dark terminal: readable ones instead
    const pies = ["#5b8def", "#d0679d", "#3fb5a3", "#e0a240", "#9b7be0", "#e06c5b", "#6bbf59", "#56b3d9"];
    const themeVariables = dark ? Object.fromEntries(pies.map((c, i) => [`pie${i + 1}`, c])) : {};
    mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme, themeVariables, htmlLabels: false,
      flowchart: { htmlLabels: false }, fontFamily: "-apple-system, BlinkMacSystemFont, Helvetica, sans-serif" });
    shownTheme = theme;
  }
  const { svg } = await mermaid.render(`wg-mermaid-${++seq}`, source);
  return svg;
}

/** The diagram as an image (an SVG data URL: nothing in it runs) at its own size. */
export async function diagramImage(source: string, dark: boolean): Promise<HTMLImageElement> {
  const root = new DOMParser().parseFromString(await mermaidSvg(source, dark), "image/svg+xml").documentElement;
  if (root.nodeName !== "svg") throw new Error("The diagram did not draw.");
  // its own size, so the image has one: Mermaid draws it at 100% width with a max-width
  const [, , w, h] = (root.getAttribute("viewBox") ?? "").split(/[\s,]+/).map(Number);
  if (w > 0 && h > 0) { root.setAttribute("width", String(w)); root.setAttribute("height", String(h)); }
  root.removeAttribute("style");
  const img = new Image();
  img.className = "diagram";
  img.alt = "Mermaid diagram";
  img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(new XMLSerializer().serializeToString(root));
  await img.decode().catch(() => {});
  return img;
}

/** Whether the panel's background is dark (its theme's, or the system's without one). */
function darkPanel(): boolean {
  const probe = document.createElement("span");
  probe.style.color = "var(--bg)";
  document.body.append(probe);
  const [r, g, b] = (getComputedStyle(probe).color.match(/[\d.]+/g) ?? []).map(Number);
  probe.remove();
  return 0.2126 * r + 0.7152 * g + 0.0722 * b < 128;
}

/** The theme changed: the documents' diagrams drawn for the other darkness are drawn again
 *  (each image keeps its fence's source and darkness). */
export async function redrawDiagrams(): Promise<void> {
  const dark = darkPanel();
  for (const img of document.querySelectorAll<HTMLImageElement>(".md-diagram img.diagram")) {
    if ((img.dataset.dark === "1") === dark) continue;
    const fresh = await diagramImage(img.dataset.source ?? "", dark).catch(() => null);
    if (!fresh || !img.isConnected) continue;
    Object.assign(fresh.dataset, { source: img.dataset.source, dark: dark ? "1" : "0" });
    fresh.hidden = img.hidden;
    img.replaceWith(fresh);
  }
}

/** A ```mermaid fence of a rendered document drawn as its diagram, with "Source" over it to
 *  show the fence and "Diagram" back; a fence Mermaid cannot draw keeps its source, Mermaid's
 *  message above it. Dark as the panel's theme (drawn again when it changes). */
export async function drawFence(pre: HTMLElement, source: string): Promise<void> {
  const dark = darkPanel();
  let img: HTMLImageElement;
  try {
    img = await diagramImage(source, dark);
  } catch (e: any) {
    const say = Object.assign(document.createElement("div"), { className: "md-diagram-error", textContent: `Mermaid: ${e?.message ?? e}` });
    pre.before(say);
    return;
  }
  if (!pre.isConnected) return;                   // the document was rendered again meanwhile
  const fig = Object.assign(document.createElement("figure"), { className: "md-diagram" });
  const toggle = Object.assign(document.createElement("button"), { type: "button", className: "md-diagram-toggle", textContent: "Source" });
  pre.replaceWith(fig);
  pre.hidden = true;
  fig.append(toggle, img, pre);
  Object.assign(img.dataset, { source, dark: dark ? "1" : "0" });
  toggle.addEventListener("click", () => {
    const shown = fig.querySelector<HTMLImageElement>("img.diagram")!, source = shown.hidden;
    shown.hidden = !source; pre.hidden = source;
    toggle.textContent = source ? "Source" : "Diagram";
  });
}
