// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The bar between the tree and the viewer: dragged, it sets their split (a share of the region
// below the tree's top); a double click sets it back to half. Each change ends in `done`.

export function initSplitter(
  sp: HTMLElement, region: () => { top: number; bottom: number }, set: (share: number) => void, done: () => void,
) {
  sp.addEventListener("pointerdown", (e) => {
    sp.setPointerCapture(e.pointerId);
    const { top, bottom } = region();
    const move = (ev: PointerEvent) => set((ev.clientY - top) / (bottom - top));
    // a release, a cancelled gesture (the system took the touch) or a lost capture ends it once
    const ends = ["pointerup", "pointercancel", "lostpointercapture"] as const;
    const end = () => { sp.removeEventListener("pointermove", move); for (const t of ends) sp.removeEventListener(t, end); done(); };
    sp.addEventListener("pointermove", move);
    for (const t of ends) sp.addEventListener(t, end);
  });
  sp.addEventListener("dblclick", () => { set(0.5); done(); });
}
