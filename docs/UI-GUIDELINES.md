# terminum UI guidelines

Binding on humans and AI for any change to the Page (`web/static/`) or the Panel (`ui/src/`).
Behavior lives in the specs; this file says what the UI looks like, what to reuse, and what to
check before a UI change is done. Where the code disagrees, it moves to this file when that area
is next changed. CLAUDE.md and the specs win over this file.

## 1. Principles

| Principle | Rule |
| --- | --- |
| Terminal first | The terminal gets the space; chrome is one header row, one list, floating buttons only where the thumb is. Nothing covers terminal text without a way to dismiss it. |
| IDE-standard | Controls look and behave like VS Code / iTerm2 / macOS: segmented controls, ghost icon buttons, popover menus, modal dialogs for irreversible actions. No novel widgets. |
| Fail loud, in place | An error shows where the action was taken (form line, banner, toast) with terminumd's words; never only in the console. |
| Phone = finger-sized | Under `(max-width: 860px)` or `(pointer: coarse)` every tappable target is ≥ 44 × 44 px (≥ 40 inside a list row whose height is 40); text fields keep their role's size (§4.6: the viewport's `maximum-scale=1` stops iOS zooming in on focus). |
| Theme-driven colors | Surfaces, text, lines and accent derive from the session's theme (`--t-bg`, `--t-fg`, `--accent`). Fixed hues only for meaning that must not change with the theme (§2.2), and they must read on a light theme too. |
| One look per kind | Two controls of the same kind share one class and one rule. Style by class; ids are for behavior and tests. |
| Calm motion | Transitions ≤ 200 ms; every animation has a `prefers-reduced-motion: reduce` off-switch. |

## 2. Tokens

### 2.1 Theme colors (`web/static/style.css :root`; set per session by app.js)

| Token | Use |
| --- | --- |
| `--t-bg`, `--t-fg` | Terminal background and text; the base of every mix. |
| `--chrome` | Sidebar, header, menu (`fg 6%` over bg). |
| `--chrome-2` | Raised surfaces: popovers, footer forms, fields, hover fill (`fg 12%`). |
| `--line` | Every 1px divider and control edge (`fg 16%`). |
| `--muted` | Secondary text, idle icons, placeholders (`fg 58%`). |
| `--accent` | The theme's cursor color: selection, focus ring, primary button fill, active tab. |
| `--sel`, `--selfg` | Terminal text selection. |
| `--size` | Terminal font size; also dialogs, footer forms and their fields, find box (tmux.css). |
| `--ui`, `--term-font` | Chrome font; terminal font (also forms and dialogs, §4.6). |

The Panel has its own names (`ui/src/style.css`: `--bg`, `--bg-2`, `--bg-hover`, `--fg-dim`,
`--fg-faint`, `--line`, `--accent`, `--ok`, `--warn`, `--bad`); under `html.themed` they map onto
the same theme. Map: `--chrome`≈`--bg-2`, `--chrome-2`≈`--bg-hover`, `--muted`≈`--fg-dim`.

Hover fill inside a `--chrome-2` popover: `color-mix(in srgb, var(--t-fg) 10%, var(--chrome-2))`.
Selected list row: `color-mix(in srgb, var(--accent) 16%, var(--chrome))`. Pressed segment:
`color-mix(in srgb, var(--accent) 22%, var(--chrome-2))`.

### 2.2 Fixed hues (by meaning)

| Meaning | Token / value | Where |
| --- | --- | --- |
| Bookmark | `--mark: #ff9f1a` (marks.css) | Everything about bookmarks, its dialog's primary. |
| This Mac / a host | `--local: #4f9dff`, `--remote: #a98bff`; `--host-label: cornflowerblue` | Rows, group labels, `.tsess` pill. |
| Danger / error | `--danger: #ff6b5e` (as text mixed: `color-mix(in srgb, var(--danger) 80%, var(--t-fg))`), `--danger-fill: #b3372e` (fills, white text) | Remove/Close/Sign out, error lines, `#status`, confirm primary, armed delete. |
| Done / ok | `--ok-fill: #2f7d4f` | `#restored`, update chip. |
| Warning / waiting | `--warn-fill: #9a6a12`; `--waiting: #ffb020` (a dot or edge, never text) | `#stall`, `#sized`, Screen label warn, waiting cards. |
| Text on the bookmark color | `#1b1200` | `#markbox .primary`. |
| Picture viewer, find highlight | fixed white on black | `#zoom`, find marks: they sit over any theme. |
| Agent words | `.ag-claude`, `.ag-codex`, `.state.working` | Mix with `--t-fg` (not raw pastel) so they read on light themes. |

Rules: a new fixed hue needs a meaning not yet in this table; define it once as a token in
`:root` (style.css), never repeat the hex. A fixed hue used as text is mixed with `--t-fg`; check
it on the light theme. Fills carrying white text reach 4.5:1 on their own (do not mix them with
`--t-bg`: on a light theme that washes them out). Shadows and backdrops use `#000` at 35–50%
(`color-mix(in srgb, #000 40%, transparent)`); that is the one raw color allowed everywhere.

### 2.3 Type scale (`--ui`, body 14px/1.35)

| Size | Weight | Use |
| --- | --- | --- |
| 22px | 600 | Phone sheet title ("Sessions"). 26px only on the sign-in page. |
| 15px | 600 | Header title; dialog title; phone footer buttons and row titles. |
| 14px | 500 | Footer buttons, menu rows, `.winhead`. |
| 13.5px | 500 | List row title (wide); phone group header. |
| 13px | 400 | Toast, waiting cards, filter, bookmark bar. |
| 12.5px | 600 / 500 | Group header, segment buttons, form hints and errors, chips. |
| 12px | 400 | Secondary lines (`.s`, menu `small`), counts, find status. |
| 11px | 400–600 | Counts, `kbd`, tabular numbers. Never smaller except badges (9.5px). |

Weights: 400 body, 500 controls and titles, 600 headings and primary buttons. No 700 in chrome.
Numbers that change (counts, sizes, positions) use `font-variant-numeric: tabular-nums`.

### 2.4 Spacing, radii, heights

Spacing steps: 2, 4, 6, 8, 10, 12, 16 px. Sidebar and header side padding 10px (wide), 16px on
the phone sheet. Gaps between sibling controls: 6px (list, footer, forms), 8px (header tools).

| Radius | Use |
| --- | --- |
| 6–7px | Small controls ≤ 28px: segment buttons, group-header buttons, form buttons, menu items. |
| 8px | Controls 30–36px: icon buttons, footer buttons, fields, list rows (wide). |
| 10px | Popovers, footer forms, bookmark bar, dialog buttons. |
| 12px | Phone controls ≥ 44px (icon buttons, footer buttons, rows, search); the ⋯ menu. |
| 14px | Dialogs. |
| 999px | Pills: toast, chips, `.tsess`, Screen label. |

| Control | Wide | Phone / coarse |
| --- | --- | --- |
| Header icon button `.icon` | 32 | 44 |
| Footer button `.footbtn` | 36 | 48 |
| Group header button (copy, import, +, ⋯) | 26 | 40 |
| Group header row / toggle | 30 / 28 | 40 / 40 |
| List row `.item` | ≥ 52 | ≥ 60 |
| Segment button | 26 (header `#views` 30) | 34 |
| Form button (footer form, find box) | 32 | 44 |
| Dialog button | 40 | 44 |
| Popover menu item | 34 | 44 |
| Floating button (`#fabs`, `#bottom`) | 40 | 44 |

### 2.5 Icons

- Lucide only: inline `<svg class="lu" viewBox="0 0 24 24" aria-hidden="true">` in index.html, or
  `icon(name, size)` from `web/static/icons.js` (Panel: `ui/src/icons.ts`, `ui/src/icons/`). Stroke 2,
  round caps and joins, `currentColor`. Add a new glyph to `ICONS` from lucide-static, never draw one.
- Size by `--s`: 14px inside rows, segments and group headers; 15–16px in footer buttons,
  popovers and icon buttons; 18px in the header's ☰/⋯ and the phone floating buttons. 17px only for
  framed glyphs (rect outlines) beside 18px ones.
- An icon-only button has `aria-label`; a `title` when its meaning is not obvious.

### 2.6 States

| State | Look |
| --- | --- |
| Hover | Ghost buttons get `--chrome-2` fill and `--t-fg` text; never change size or weight. Wrap new hover rules in `@media (hover: hover)` so a tap does not leave them on. |
| Pressed toggle (`aria-pressed="true"`) | `--chrome-2` fill (header tools) or accent mix (segments, §2.1). |
| Expanded (`aria-expanded="true"`) | Same as hover, held while open (footer buttons, ⋯, host menu button). |
| Selected (`aria-selected`, `.on`) | Accent mix fill; wide `#views`: accent text and accent edge. |
| Disabled | `opacity: .35–.4; cursor: default`. |
| Focus | `:focus-visible` only: 2px `--accent` ring (shapes.css draws it as the ring for clipped buttons; `--focus: var(--t-bg)` on an accent-filled button). Never `outline: none` without a replacement. |
| Busy | `.busy` breathing or `Searching…` text; no spinners in the Page. |

### 2.7 Shapes (`web/static/shapes.css`)

Every `<button>` is clipped to a slightly uneven outline and its border is drawn by `--edge`
(never `border`, which the clip cuts). Set `--edge: var(--line)` for an outlined button,
leave it unset for a ghost button. Round floating buttons are excluded by id in shapes.css and use
the `blob` radius; a new floating button is added to that exclusion list.

## 3. Components

Rule: style by a shared class, never by a single `#id` when a sibling of the same kind exists.
Before adding a rule, find the sibling's class and reuse it; add a modifier only for a real
difference.

Rules shared by kind live in `web/static/controls.css` (loaded after the feature sheets): dialog
actions, group-header buttons, popover items, the phone's touch sizes. A rule for a kind goes
there, not into the file of the feature that first needed it.

| Component | Class / markup | CSS | Notes and "do not" |
| --- | --- | --- | --- |
| Footer button | `button.footbtn` > `svg.lu --s:15px` + `span` | style.css | Same look for all four; a count is `span.count`. Do not add per-id size, color or padding (the icon's color may vary by meaning, e.g. `#marksbtn > svg`). |
| Footer form | `form.footform` + `p.hint` + `p[role=alert]` + `.row` > `button.primary` + Cancel | tmux.css | One open at a time; opens above the footer; primary first, Cancel second. Error text only in the `role="alert"` line. |
| Header icon button | `button.icon` | style.css | Ghost; 32/44px. Pressed or expanded state via `aria-*`, not a class. |
| Segmented control | `div.seg` > `button[aria-pressed]` or `[role=tab][aria-selected]` | style.css | Lines, Screen mode, Views. Text labels, 12.5px. |
| Popover menu | `div[role=menu]` > `button` | tmux.css (`#hostmenu`, `#updmenu`), controls.css (items), style.css (`#menu`) | `--chrome-2`, 1px `--line`, radius 10, padding 4, shadow; items left-aligned, 34px; destructive item in `--danger` (mixed) and last. A new popover joins the `:is(#hostmenu, #updmenu)` item rules. |
| Settings menu row | `#menu .row` (button or label) with `small` description | style.css | Label left, control right; danger row last with a divider. |
| Dialog | `<dialog>` > `form[method=dialog]` > `.actions` (Cancel, then primary) | controls.css (actions), style.css, tmux.css, marks.css (boxes) | `--chrome`, radius 14, padding 16, width ≤ 480px; primary right; destructive primary in `--danger-fill` (`#confirmbox`). Use `askConfirm(text, verb)` (navtools.js) for every irreversible action. |
| Primary button | `button.primary` | per container | Fill `--accent`, text `--t-bg`, weight 600, `--focus: var(--t-bg)`. Danger: `--danger-fill` + white. Bookmark: `--mark`. Make sure no container rule out-ranks it (test the computed fill). |
| Banner | `div[role=status]` under the header (`#status`, `#restored`, `#stall`, `#sized`) | style.css | Full width, 6px 12px, white on a fixed fill (§2.2); a close `.x` when it does not clear itself. |
| Toast | `#toast` via `toast(text, ms)` (app.js) | style.css | One line when short, wraps when long, never wider than the terminal column minus 32px; above the floating buttons on a phone. |
| Waiting card | `#asks .ask` > `.go` + `.x` | style.css | Starts under the banners and the find button (wide: `grid-row: 6 / -1`). |
| List row | `button.item` (`.pbar`, `.txt` > `.head` + `.s`, `.st`) | style.css | Title ellipsis, meta line gives way from its end. Child rows `.item.child`. |
| Group header | `.grp h2` > `.grip`, `button.toggle`, `.attach`, `.import`, `.new`, `.hostbtn` | style.css, controls.css | The trailing buttons share size, radius and color (controls.css); `.new` keeps its edge as the group's own action. The name keeps 6 characters; the session pill gives way first. |
| Badge / pill | `.tsess`, `.count`, `#scrlabel`, `#updchip`, `#marksfabn` | style.css, tmux.css, marks.css | Counts: muted 11px tabular; pills 999px. |
| Floating button | `#fabs > button`, `#findfab`, `#marksfab`, `#bottom` | style.css, find.css, marks.css | 44px on touch, blob radius, blurred `--chrome-2`; exclude from shapes.css. |
| Grip / splitter | `.grip` (rows), `[role=separator]` (`#navgrip`, `#quickgrip`) | style.css, tmux.css | Keyboard-operable (arrow keys), accent on hover/focus. |
| Window head | `.winhead` | style.css | Title 14px/600 left, `.icon` close right. |
| Panel tree row | `.row` (`.guides`, `.chev`, `.ico`, `.name`, `.size`) | ui/src/style.css | Height `--row`; selection `--bg-sel`; keyboard cursor inset ring. |
| Panel tabs | `.tab` (`.tname`, `.close`), `.tab.active::after` accent bar | ui/src/style.css | Dirty: accent dot in `.close`. |
| Panel buttons, dialogs, menus | `.btn` (`.primary`, `.danger`, `.sm`), `.modal`, `.ctx .mi` | ui/src/style.css | Same roles as the Page's; do not invent a third style. |

## 4. Copy

1. **Buttons**: verb + object, sentence case, ≤ 3 words, ≤ 20 characters ("New session", "Add host",
   "Split right", "Copy upgrade command"). A dialog's primary repeats the verb of the question
   ("Remove", "Close"), never "OK" or "Yes".
2. **Tooltips** (`title`): say what happens or what it is, in a sentence fragment; no trailing period;
   do not repeat the label.
3. **Errors**: terminumd's message verbatim where it has one; otherwise what failed and what to do
   ("The Mac does not answer."). Full sentences with a period.
4. **Placeholders**: an example or the field's name; must fit the field at 300px sidebar width.
5. **Names in UI text** (labels, tooltips, errors):

| Thing | Say | Not |
| --- | --- | --- |
| tmux session (a group) | session | workspace, tab |
| tmux window (a row) | window | tab (except "tab's panes" in code comments) |
| tmux pane | pane | split, terminal |
| Another computer over ssh | host | server, remote, machine |
| The user's default tmux server | your tmux | system tmux, plain tmux |
| terminum's own tmux server | terminum's tmux | private server (allowed in docs) |
| Showing a session of the user's tmux | attach (detach) | import, add |
| This computer | this Mac | local, localhost |

6. **Fonts by role**: dialogs, footer forms, the find box and waiting cards use `--term-font` at
   `--size` (tmux.css; popups follow the terminal's size), fields too: the viewport's
   `maximum-scale=1` keeps iOS from zooming in on a field below 16px. Chrome
   (header, list, menus, footer buttons, banners, toast) uses `--ui` at the §2.3 scale. A new
   popup follows the first rule.

## 5. Adding UI

1. Find the closest existing component in §3 and its class; reuse it unchanged if possible.
2. Markup in `index.html` (static) or built in a module with `el()`/`icon()`; ids only for wiring
   and tests.
3. CSS in the file that owns the area (style.css: shell, list, header, terminal; tmux.css:
   terminum's additions, forms, dialogs; marks.css, find.css, widgets.css: their features;
   shapes.css: button outlines only). Respect the 500-line cap; split by feature, not by squeezing.
4. Tokens only (§2); no new font size, radius or height outside the scales.
5. Phone: add the finger size in controls.css's `max-width: 860px` block (≥ 44px; 40 inside a 40px row; a segment 34).
6. States: hover, focus-visible, pressed/expanded, disabled; `aria-*` drives them.
7. Copy per §4; add the words to the spec's scenario first when behavior changes (CLAUDE.md §7).

## 6. Change checklist

A UI change is done only when every line it can affect holds; say in the delivery which
screenshots you looked at. A copy-only change needs §4 and one screenshot.

- [ ] Screenshot of the changed area **next to its siblings**: same height, padding, radius, fill,
      edge, font, icon size and alignment.
- [ ] Chromium **and** WebKit, at 1400×900 and 390×844 (touch), from `web/test/fake.mjs`.
- [ ] A dark and a light theme (in `openPage`'s `hook`, answer `sub` with `say({ t: "theme", sid: m.id, theme: { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false } })`): text and fixed hues readable
      (≥ 4.5:1 for text, ≥ 3:1 for icons and edges).
- [ ] Touch targets ≥ 44px on the phone (measure `getBoundingClientRect`, not the CSS).
- [ ] Keyboard: reachable by Tab, visible focus ring, Esc closes popovers and forms.
- [ ] Hover, pressed, expanded and disabled states; no hover left on after a tap.
- [ ] Long text: a long session/host name, a long error and toast, a 300px and a 640px sidebar;
      nothing overflows the viewport or covers what it should not.
- [ ] Empty and error states (no sessions, no bookmarks, a failed host, terminumd's refusal).
- [ ] `--size` at 10 and 20: forms, their fields and dialogs follow it, on a touch screen too.
- [ ] Banners, waiting cards, toast and find box shown together do not overlap.
- [ ] No new raw hex outside §2.2 in the changed lines (a hex you touch becomes its token); no rule styling one `#id` where a sibling class exists.
- [ ] Files ≤ 500 lines; `node web/test/e2e_all.mjs` and, for the Panel, `cd ui && node test/e2e_panel.mjs` pass.

Automated layout checks (Chromium and WebKit, both sizes): `web/test/e2e_look.mjs` (a dialog's
primary fill differs from Cancel's; a group header's buttons share size, radius and color; on a
phone every button of the list, a form, the header and the menu ≥ 40px, a segment ≥ 34px; a long
toast stays on the screen; a waiting card covers neither banners nor the find button) and
`web/test/e2e_tmux.mjs` (footer buttons share computed look; form font sizes follow `--size`). A
new kind of control adds its check there. The other e2e checks test behavior, not look.
