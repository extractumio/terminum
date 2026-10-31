// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The session list's foot (AC-07; its look: foot.css). On a computer it is folded to one row of
// its buttons' icons, the bookmarks' count after theirs, with ▸: one toggle button; a click
// unfolds it (▾) and it stays until clicked again, remembered for the next load. Folded, its
// buttons are out of the keyboard's way; folding closes a form of theirs as Esc does. A touch
// screen or a narrow window shows them as a row of blobs, never folded. Under them: the build.

const $ = (id) => document.getElementById(id);
const FOLDS = matchMedia("(min-width: 861px) and (pointer: fine)");
const FORMS = ["newsess", "addhost", "addtmux"];

/** store: the page's settings (app.js: a split's embedded page keeps none). */
export function listFoot({ store }) {
  const foot = document.querySelector(".nav-foot"), toggle = $("foottoggle"), menu = $("footmenu");
  const icons = toggle.querySelector(".icons");
  for (const b of foot.querySelectorAll(".footbtn")) {
    const icon = b.querySelector("svg").cloneNode(true);
    if (b.id !== "marksbtn") icons.append(icon);
    else { const mk = document.createElement("span"); mk.append(icon, $("markpeek")); icons.append(mk); }
  }
  let open = store.get("footOpen", false) === true;
  function sync() {
    const folded = FOLDS.matches && !open;
    foot.classList.toggle("open", open);
    toggle.setAttribute("aria-expanded", String(!folded));
    menu.inert = folded;
  }
  toggle.addEventListener("click", () => {
    open = !open;
    // a form of the foot's open (its button pressed): folding closes it, as Esc does
    if (!open) for (const f of FORMS) if ($(`${f}btn`).getAttribute("aria-expanded") === "true") $(`${f}cancel`).click();
    store.set("footOpen", open);
    sync();
  });
  // the remembered state is drawn at once, never folded or unfolded in sight
  foot.classList.add("instant");
  sync();
  requestAnimationFrame(() => requestAnimationFrame(() => foot.classList.remove("instant")));
  FOLDS.addEventListener("change", sync);
}

/** The build under the session list, small and quiet: a release's version, else "dev" and its id. */
export function showBuild(m) {
  const id = String(m.id ?? ""), version = /^v\d/.test(id) ? id : `dev ${id.slice(0, 7)}`;
  $("buildinfo").textContent = ["terminum", version, m.date].filter(Boolean).join(" · ");
  $("buildinfo").title = `Build ${id}${m.date ? `, ${m.date}` : ""}`;
}
