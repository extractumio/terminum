// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's comments and history in its window (terminum-kanban.md AC-107, AC-108, AC-149), read as a conversation:
// a comment a card with a bar at its left in the color of its kind (a human's, an agent's, Dispatcher's, an agent's
// questions), a line of history muted between them. What the task waits for the human to react to (its
// agent's questions, its result, why it failed) is set apart to stand first, the rest then newest first; with
// nothing to react to, all of it reads oldest first.
import { el, iconButton } from "./listparts.js";
import { api, button, renderInto, row, say, shortStamp, stamp } from "./kanbanutil.js";
import { expandable } from "./expand.js";

const OPENS = "\n\n```\n", CLOSES = "\n```";   // round a screen quoted in a line of history

/** A field for Markdown. */
export function editor(text, label) {
  const area = el("textarea", "field");
  area.value = area.defaultValue = text;
  if (label) area.setAttribute("aria-label", label);
  return area;
}

/** What `t` waits for its human to react to, with its heading: the questions it waits with, a finished task's
 *  result (its agent's last comment), a failed one's last word from its agent or Dispatcher; none: null. */
function reactsTo(t) {
  const last = (heading, fits) => {
    const e = t.history.findLast((h) => h.kind === "comment" && fits(h));
    return e ? { e, heading } : null;
  };
  if (t.status === "WAIT FOR HUMAN") return last("Asks you", (e) => e.questions);
  if (t.status === "DONE") return last("Result", (e) => e.author_kind === "agent");
  if (t.status === "FAILED") return last("Why it failed", (e) => e.author_kind !== "human");
  return null;
}

/** `refresh(saved)` reads the task again; `confirmed(question, yes, run)` asks, then runs. Answers `lead`: the part
 *  that stands first, or null (always with `leads` off: all of it reads in order); and `rest`: the conversation,
 *  without what leads. */
export function activity(t, { refresh, confirmed, leads = true }) {
  /** A human's own comment changes or goes (AC-108): small keys on its head's line. */
  function commentActions(e, said) {
    const route = `/api/tasks/${t.id}/comments/${e.id}`;
    const edit = iconButton("icon", "pencil", "Edit", "Edit this comment", () => {
      const area = editor(e.text ?? "");
      const save = button("primary", "Save", async () => {
        try {
          await api(route, { method: "PATCH", body: { text: area.value } });
          refresh(true);
        } catch (err) { say(err.message); }
      });
      said.replaceChildren(expandable(area), row(button(null, "Cancel", () => refresh(true)), save));
      area.focus();
    });
    const del = iconButton("icon", "x", "Delete", "Delete this comment", () => confirmed("Delete this comment?", "Delete", () => api(route, { method: "DELETE" })));
    const keys = el("span", "evacts");
    keys.append(edit, del);
    return keys;
  }

  function eventRow(e) {
    const li = el("li", `ev ev-${e.kind} by-${e.author_kind}${e.questions ? " ev-questions" : ""}`);
    const head = el("div", "evhead"), at = el("time", null, shortStamp(e.at));
    at.title = stamp(e.at);
    at.dateTime = new Date(e.at * 1000).toISOString();
    // a line of history says who did it in its own words ("alex made it"): its time alone before it
    const who = el("strong", null, e.author_name);
    if (e.kind === "comment") head.append(who, at);
    else head.append(at, ...((e.text ?? "").startsWith(e.author_name) ? [] : [who]));
    // an agent's questions for the human: answered in a comment, then Run (terminum-dispatcher.md AC-157)
    if (e.questions) head.append(el("span", "evasks", "asks you"));
    if (e.edited_at) head.append(el("span", "evedited", `edited ${shortStamp(e.edited_at)}`));
    li.append(head);
    if (e.kind === "comment") {
      const said = el("div", "markdown");
      renderInto(said, e.text ?? "");
      li.append(said);
      if (e.author_kind === "human") head.append(commentActions(e, said));
    } else if (e.kind === "deleted") {
      li.append(el("em", null, "Comment deleted"));
    } else {
      // its words, and the screen Dispatcher quotes after them between ``` lines (dispatcher/start.rs), as it stood
      const text = e.text ?? "", open = text.indexOf(OPENS), end = text.length - CLOSES.length;
      const quoted = open >= 0 && text.endsWith(CLOSES) && end >= open + OPENS.length;
      li.append(el("span", "evtext", quoted ? text.slice(0, open) : text));
      if (quoted) {
        li.append(expandable(el("pre", "evscreen", text.slice(open + OPENS.length, end))));   // scrolled by the keys too (expand.js)
      }
    }
    return li;
  }

  function part(cls, heading, events) {
    const wrap = el("section", `tasksec ${cls}`), head = el("div", "sechead"), list = el("ol", "history");
    head.append(el("h3", null, heading));
    list.append(...events.map(eventRow));
    wrap.append(head, list);
    return wrap;
  }

  const first = leads ? reactsTo(t) : null, others = t.history.filter((e) => e !== first?.e);
  const comments = others.filter((e) => e.kind === "comment").length;
  return {
    lead: first ? part("leads", first.heading, [first.e]) : null,
    rest: first ? part("activity", `Earlier ${comments} · newest first`, others.reverse()) : part("activity", `Conversation ${comments}`, others),
  };
}
