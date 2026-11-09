// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The page's small questions, on the session page and the board: a yes or no, one of several, and a name.
import { el } from "./listparts.js";

const $ = (id) => document.getElementById(id);

// Every dialog's ×, as Esc: a dialog that refuses Esc (the bookmark's while it saves) refuses it too.
// The bookmarks' windows close their own way (marks.js, markdoc.js).
for (const x of document.querySelectorAll("dialog.dlg .dhead .x:not([id])")) {
  x.onclick = () => { const d = x.closest("dialog"); if (d.dispatchEvent(new Event("cancel", { cancelable: true }))) d.close("cancel"); };
}

// One question at a time in a box: asked while it is open, a question waits for that one's answer, so one
// Enter answers one (ui/src/dialogs.ts does so for the Files panel's).
const queues = new Map();
function inTurn(id, open) {
  const turn = (queues.get(id) ?? Promise.resolve()).then(open);
  queues.set(id, turn);
  return turn;
}

/** The yes-or-no dialog, open with this title and text. */
function openConfirm(title, more) {
  const box = $("confirmbox");
  $("confirmtitle").textContent = title;
  $("confirmtext").textContent = more ?? "";
  $("confirmtext").hidden = !more;
  box.returnValue = "";
  box.showModal();
  return box;
}

/** Asks `text` with a dialog: true when the user pressed `yes` (else Cancel, ×, Esc). Its question
 *  ("Close the session api?") is the title, what follows it the text. */
export function askConfirm(text, yes) {
  const [, title = text, more = ""] = /^(.+?\?)\s+(.+)$/s.exec(text) ?? [];
  return inTurn("confirmbox", () => {
    $("confirmok").textContent = yes;
    const box = openConfirm(title, more);
    $("confirmok").focus();
    return new Promise((resolve) => box.addEventListener("close", () => resolve(box.returnValue === "ok"), { once: true }));
  });
}

/** Several ways on: the value of the button pressed, or null for Cancel and Esc. */
export function choose(title, more, options) {
  return inTurn("confirmbox", () => chooseNow(title, more, options));
}

function chooseNow(title, more, options) {
  $("confirmok").hidden = true;
  const added = options.map(([value, label]) => {
    const b = el("button", null, label);
    b.type = "submit";
    b.value = value;
    return b;
  });
  $("confirmacts").append(...added);
  const box = openConfirm(title, more);
  return new Promise((resolve) => box.addEventListener("close", () => {
    $("confirmok").hidden = false;
    added.forEach((b) => b.remove());
    resolve(box.returnValue && box.returnValue !== "cancel" ? box.returnValue : null);
  }, { once: true }));
}

/** Asks for a name with a dialog: the text typed (trimmed), or null for Cancel and Esc; `yes`
 *  names its button. Enter in the field is the button (the form's first, Cancel, would be). With
 *  `save`, the dialog stays until it took the name; a throw shows its message in `#nameerror`; while it
 *  saves, Enter again sends nothing and Esc, × and Cancel wait. */
export function askName(text, value, yes = "Rename", most = 40, save = null) {
  return inTurn("namebox", () => askNameNow(text, value, yes, most, save));
}

function askNameNow(text, value, yes, most, save) {
  const box = $("namebox"), field = $("namefield"), error = $("nameerror"), form = box.querySelector("form");
  $("nametext").textContent = text;
  $("nameok").textContent = yes;
  field.maxLength = most;
  field.value = value;
  if (error) error.hidden = true;
  let saving = false;
  const answer = async () => {
    if (saving) return;
    saving = $("nameok").disabled = true;
    try {
      await save?.(field.value.trim());
      box.close("ok");
    } catch (e) {
      error.textContent = e.message;
      error.hidden = false;
    } finally { saving = $("nameok").disabled = false; }
  };
  const submit = (e) => {
    if (saving) e.preventDefault();                 // Cancel waits for the save, as Esc does
    else if (save && e.submitter?.value === "ok") { e.preventDefault(); answer(); }
  };
  const wait = (e) => { if (saving) e.preventDefault(); };
  field.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); answer(); } };
  form.addEventListener("submit", submit);
  box.addEventListener("cancel", wait);
  box.returnValue = "";
  box.showModal();
  field.select();
  return new Promise((resolve) => box.addEventListener("close", () => {
    form.removeEventListener("submit", submit);
    box.removeEventListener("cancel", wait);
    resolve(box.returnValue === "ok" ? field.value.trim() : null);
  }, { once: true }));
}
