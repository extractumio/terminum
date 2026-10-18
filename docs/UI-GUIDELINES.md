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
| Surfaces, not lines | Elements are set apart by space or by a step of background (§2.1); a 1px line only for a tree guide or rendered content. |
| Drawn by hand | Every box's horizontal edges bow a little and no round thing is a circle (§2.7). |
| Calm motion | Feedback ≤ 120 ms, a box comes in ≤ 200 ms, a click's ripple fades ≤ 320 ms after the action (it never delays it); nothing moves in the terminal; one `prefers-reduced-motion: reduce` off-switch (§2.8). |

## 2. Tokens

The design code is `web/static/tokens.css` (loaded first; the Panel imports it): every step,
surface, size, shape and timing by name. A rule uses a token, never a new value; a new value is
added there first, with its role, and here.

### 2.1 Theme colors and surfaces (`tokens.css`; the theme by app.js on `:root`)

Elevation is a step of background, never a line: each surface is `--t-fg` mixed into `--t-bg`.

| Token | Use |
| --- | --- |
| `--t-bg`, `--t-fg` | Terminal background and text; the base of every mix (style.css defaults, app.js per session). |
| `--surface-0` | `--t-bg`: the terminal, Files, View. |
| `--surface-1` = `--chrome` | Sidebar, header, menu, dialogs (`fg 6%`). |
| `--surface-2` | Raised on the chrome: secondary buttons, the segment track, row hover (`fg 9%`). |
| `--surface-3` = `--chrome-2` | Floating: popovers, footer forms, fields on the chrome, hover on the chrome (`fg 12%`). |
| `--surface-4` | Hover and pressed on a floating surface; Cancel in a form (`fg 18%`). |
| `--line` | The few lines left: tree guides (`.kids`, the Panel's guides), rendered content, a field's resting ring on a light theme (`fg 16%`). |
| `--muted`, `--faint` | Secondary text, idle icons, placeholders (`fg 58%`); a quiet dot (`fg 40%`). |
| `--shadow-near`, `--shadow-far` | A floating button's / a box's shadow, 3px inside the box (§2.7); `#000` at `--shadow-a` (40%, 18% on a light theme). |
| `--accent` | The theme's cursor color: selection, primary button fill, active tab. |
| `--ring` | The focus ring: `--accent`, or `--t-fg` when the accent is under 3:1 against `--t-bg` (contrast.js). |
| `--sel`, `--selfg` | Terminal text selection. |
| `--size` | Terminal font size; also dialogs, footer forms and their fields, find box (tmux.css). |
| `:root.light` | Set by app.js when the theme says `dark: false`: lighter shadows, darker folder hues, fields get a resting ring. |
| `--ui`, `--term-font` | Chrome font; terminal font (also forms and dialogs, §4.6). |

The Panel has its own names (`ui/src/style.css`: `--bg`, `--bg-2`, `--bg-hover`, `--fg-dim`,
`--fg-faint`, `--line`, `--accent`, `--ok`, `--warn`, `--bad`); under `html.themed` they map onto
the same theme. Map: `--chrome`≈`--bg-2`, `--chrome-2`≈`--bg-hover`, `--muted`≈`--fg-dim`.

Hover fill inside a `--chrome-2` popover: `--surface-4`.
Selected list row: `color-mix(in srgb, var(--accent) 16%, var(--chrome))`. Pressed segment:
`color-mix(in srgb, var(--accent) 22%, var(--chrome-2))`.

### 2.2 Fixed hues (by meaning)

| Meaning | Token / value | Where |
| --- | --- | --- |
| Bookmark | `--mark: #ff9f1a` (marks.css) | Everything about bookmarks, its dialog's primary. |
| This Mac / a host | `--local: #4f9dff`, `--remote: #a98bff`; `--host-label: cornflowerblue` | Rows, group labels, `.tsess` pill. |
| Danger / error | `--danger: #ff6b5e` (a dot or icon), `--danger-text` (= danger 80% into `--t-fg`: every error text), `--danger-fill: #b3372e` (fills, white text) | Remove/Close/Sign out, error lines, a failed host's state, `#status`, confirm primary, armed delete. |
| Done / ok | `--ok-fill: #2f7d4f` | `#restored`, update chip. |
| Warning / waiting | `--warn-fill: #9a6a12`; `--waiting: #ffb020` (a dot or edge, never text) | `#stall`, `#sized`, Screen label warn, waiting cards. |
| Text on the bookmark color | `#1b1200` | `#markbox .primary`. |
| Picture viewer, find highlight | fixed white on black | `#zoom`, find marks: they sit over any theme. |
| Agent words | `.ag-claude`, `.ag-codex`, `.state.working` | Mix with `--t-fg` (not raw pastel) so they read on light themes. |
| A row's folder | `oklch(var(--dir-l) var(--dir-c) <hue>)`, the hue from `dircolors.js` (AC-01): L .80 C .085, on a light theme L .62 C .11 | The bar left of a row and of the header's title. Unrelated folders spread evenly over the hue circle; a nested folder near its parent's hue, nearer the deeper; one folder one hue. |

Rules: a new fixed hue needs a meaning not yet in this table; define it once as a token in
`:root` (style.css), never repeat the hex. A fixed hue used as text is mixed with `--t-fg`; check
it on the light theme. Fills carrying white text reach 4.5:1 on their own (do not mix them with
`--t-bg`: on a light theme that washes them out). Shadows and backdrops use `#000` at 35–50%
(`color-mix(in srgb, #000 40%, transparent)`); that is the one raw color allowed everywhere.

### 2.3 Fonts and type scale

Two families: `--ui` (the system's: SF Pro on Apple devices) for chrome at fixed sizes, and
`--term-font` (the theme's font, else Menlo) for the terminal and for what is typed (dialogs, forms,
fields, the find box: §4.6) at `--size`, which the user scales with − and +. Chrome does not scale
with `--size`; a phone gets its own steps (below). Body 14px/1.35.

| Token | Size | Weight | Use |
| --- | --- | --- | --- |
| `--fs-display` | 22px | 600 | Phone sheet title ("Sessions"). 26px only on the sign-in page. |
| `--fs-title` | 15px | 600 | Header title; dialog title; phone footer buttons and row titles. |
| `--fs-body` | 14px | 500 | Footer buttons, menu rows, `.winhead`. |
| `--fs-row` | 13.5px | 500 | List row title (wide); phone group header. |
| `--fs-small` | 13px | 400 | Toast, waiting cards, filter, bookmark bar. |
| `--fs-label` | 12.5px | 600 / 500 | Group header, segment buttons, form hints and errors, chips. |
| `--fs-meta` | 12px | 400 | Secondary lines (`.s`, menu `small`), group state lines, counts, find status. |
| `--fs-micro` | 11px | 400–600 | `kbd`, tabular numbers, the touch footer's captions. Never smaller except badges (9.5–10px). |

Weights `--fw-body` 400, `--fw-ctl` 500, `--fw-head` 600; no 700 in chrome. Line heights
`--lh-tight` 1.2 (badges), `--lh-ui` 1.35 (chrome), `--lh-prose` 1.45 (dialog text). Numbers that
change (counts, sizes, positions) use `font-variant-numeric: tabular-nums`.

### 2.4 Grid, spacing, radii, heights

A 4px grid: `--sp-0` 2 (inside a control only), `--sp-1` 4, `--sp-2` 8, `--sp-3` 12, `--sp-4` 16,
`--sp-5` 20, `--sp-6` 24. The older 6px and 10px steps stay where they are until that rule is next
changed (they become 4/8 and 8/12). `--gutter`: 12px wide, 16px on a touch screen.

| Rhythm | Value |
| --- | --- |
| Sidebar and header side padding | 10px wide, 16px on the phone sheet |
| Between groups in the list | `--sp-3` (14px on the phone) |
| Sibling controls | 4px (footer list), 8px (forms, dialog actions, header tools) |
| A form | padding 12 (16 on touch); field ↔ field 8; a hint under its field 4; the last field → its buttons 16 (20 on touch) |
| A dialog | padding 16; its text → its buttons 20 |

| Radius | Use |
| --- | --- |
| `--r-xs` 4px | `kbd`, badges, the closed-session chip. |
| `--r-sm` 6px | Small controls ≤ 28px: segment buttons, group-header buttons, form buttons, menu items. |
| `--r-md` 8px | Controls 30–40px: icon buttons, footer buttons, fields, list rows (wide). |
| `--r-lg` 12px | Popovers, footer forms, bookmark bar, waiting cards; phone controls ≥ 44px. |
| `--r-xl` 14px | Dialogs, the View window. |
| `--r-pill` | Pills: chips, `.tsess`, Screen label. |
| `--blob-a` … `--blob-d` | Every round thing (floating buttons, the touch footer, dots, badges): four uneven radii, one per neighbor; never `50%`. |

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

- Lucide only, with one exception: a coding agent's own mark (`web/static/agents/claude.svg`,
  `codex.svg`, as `<img>` through `agentMark()` in nav.js; their owners' trademarks, outside the
  licenses: `NOTICE`), beside a row's title and the header's; a plain pane shows Lucide's `terminal`.
  No other brand mark is added.
- Lucide glyphs: inline `<svg class="lu" viewBox="0 0 24 24" aria-hidden="true">` in index.html, or
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
| Focus | `:focus-visible` only: 2px `--ring` ring (shapes.css draws it as the ring for clipped buttons, as an inset ring for fields; `--focus: var(--t-bg)` on an accent-filled button; an outline 2px off a blob). Never `outline: none` without a replacement. |
| Busy | `.busy` breathing or `Searching…` text; no spinners in the Page. |

### 2.7 Shapes (`web/static/shapes.css`, shared with the Panel)

A box's horizontal edges bow out or in by a pixel or two, as drawn by hand; corners stay round,
vertical edges straight; neighbors bow differently. The outline is one property, `--wave`
(`clip-path: shape()`), computed on each element from `--r` (corner), `--t`/`--b` (how far the top
and bottom edges bow: + out, − in), `--x` (where they peak) and `--k` (how much: motion). The
amplitude shrinks as the box grows relative to its size: `--wave-s` 1px fields, `--wave-m` 1.5px
buttons, `--wave-l` 2px forms and popovers, `--wave-xl` 2.5px dialogs.

| Kind | Class | How |
| --- | --- | --- |
| Button | every `<button>` | Clipped; its border is a ring of the same shape (`::after`) in `--edge` (never `border`, which the clip cuts); the focus ring is that ring in the accent. |
| Box | `.box` (popovers, footer forms, dialogs, find box, bookmark bar, waiting cards, toast; Panel `.modal`, `.ctx`) | Its fill is `--fill`, drawn by a clipped `::before`, so its own `box-shadow` (3px inside: `--shadow-far`) survives; a box that scrolls (`.box.scrolls`) is clipped itself and has no shadow. |
| Field | `.field` (an input, a textarea, or a wrapper label such as `.search`, `.lead`; Panel `.filter`) | Bows in only, so the focus ring (inset, 2px, 3px at top and bottom) is never cut; on a light theme a resting ring in `--line`. |
| Header | `.wavebar` (`#bar`, `.winhead`, the Panel's header) | Its lower edge in three uneven troughs over the terminal's color; the fill is `--fill`. |
| Blob | `#fabs button`, `#findfab`, the bookmark buttons, `#bottom`, the touch footer's icons | A `--blob-*` radius (no clip: shadow and outline follow it); the drifting ones animate it. Excluded by id in shapes.css; a new floating button joins that list. |

Without `shape()` (Safari before 18.4) everything is straight with its radius and shadow.
`prefers-contrast: more` gives fields and boxes their edges back.

### 2.8 Motion (`web/static/motion.css`, `press.js`)

| Token | Value | Use |
| --- | --- | --- |
| `--dur-press` | 60ms | A press. |
| `--dur-hover` / `--dur-out` | 120 / 160ms | Hover in / out, color, ring. |
| `--dur-enter` | 200ms | A box or dialog comes in. |
| `--dur-settle` | 320ms | A ripple, the outline's swing back, the selected row's settle: after the action. |
| `--ease-out`, `--ease-press` | cubic-bezier | Most motion; a press. |
| `--ease-spring`, `--ease-wobble` | `linear()` (cubic fallback) | A small overshoot; one swing of a pen. |

| Moment | What moves |
| --- | --- |
| Hover (`(hover: hover) and (pointer: fine)` only) | Color: one surface step. Never the outline or the size (a sweep across a toolbar must stay calm). Rows: fill and their folder bar brighten. Blobs lift 1px and grow 6%. |
| Press | Scale .96 (.985 for wide controls and rows), the outline flattens (`--k` .15). Blobs squash to .9. |
| Release | The outline swings back once (`--ease-wobble`); a ripple spreads from the pressed point inside the outline (Enter/Space: from the middle). Blobs pulse in their own outline. |
| A box opens | Fades in and rises 6px (a popover falls 4px), from .98 (a dialog from .96) with its backdrop; it closes at once. |
| The shown session changes | Its row's fill settles from the accent, its bar grows in (`.fresh`, once). |
| A session works | Its bar in the list pulses (its color glows and fades, 1.4 s; the header's hourglass says it there). A state, not motion: nothing moves, so it stays with Reduce Motion. |
| Never | Anything in `#term`, the key bar, what is typed; width, height, position or layout; banners and error text (they show at once); a new loop. |

`prefers-reduced-motion: reduce` sets every duration to 0 and turns off scale, translate, ripple and
settle; colors still change. The browser checks run with reduced motion (fake.mjs);
`web/test/e2e_motion.mjs` checks the motion itself.

## 3. Components

Rule: style by a shared class, never by a single `#id` when a sibling of the same kind exists.
Before adding a rule, find the sibling's class and reuse it; add a modifier only for a real
difference.

Rules shared by kind live in `web/static/controls.css` (loaded after the feature sheets): dialog
actions, group-header buttons, popover items, the phone's touch sizes. A rule for a kind goes
there, not into the file of the feature that first needed it.

| Component | Class / markup | CSS | Notes and "do not" |
| --- | --- | --- | --- |
| Footer button | `button.footbtn` > `svg.lu --s:15px` + `span` (label, its tail in `span.more`) | style.css | Same look for all four; a count is `span.count`. Do not add per-id size, color or padding (the icon's color may vary by meaning, e.g. `#marksbtn > svg`). On a touch screen or below 860px: one row of four blobs (48px icon fill, a different `--blob-*` each), the label's first words under each as a caption (`.more` hidden, so the name stays whole), the bookmark count a badge. |
| Footer form | `form.footform.box` + `p.hint` + `p[role=alert]` + `.row` > `button.primary` + Cancel | tmux.css | One open at a time; opens above the footer; primary first, Cancel second; fields `.field`. Error text only in the `role="alert"` line. |
| Header icon button | `button.icon` | style.css | Ghost; 32/44px. Pressed or expanded state via `aria-*`, not a class. |
| Segmented control | `div.seg` > `button[aria-pressed]` or `[role=tab][aria-selected]` | style.css | Lines, Screen mode, Views. Text labels, 12.5px. |
| Popover menu | `div.box[role=menu]` > `button` | tmux.css (`#hostmenu`, `#updmenu`), controls.css (items), style.css (`#menu`) | `--fill: var(--chrome-2)`, `--r-lg`, padding 4, `--shadow-far`, no border; items left-aligned, 34px, hover `--surface-4`; destructive item in `--danger-text`, last, set apart by 8px; Esc closes it. A new popover joins the `:is(#hostmenu, #updmenu)` item rules. |
| Settings menu row | `#menu .row` (button or label) with `small` description | style.css | Label left, control right; danger row last with a divider. |
| Dialog | `<dialog class="box">` > `form[method=dialog]` > `.actions` (Cancel, then primary) | controls.css (actions, backdrop), style.css, tmux.css, marks.css (boxes) | `--fill: var(--chrome)`, `--r-xl`, padding 16, no border, `--shadow-far`, width ≤ 480px; primary right; destructive primary in `--danger-fill` (`#confirmbox`). Use `askConfirm(text, verb)` (navtools.js) for every irreversible action. |
| Primary button | `button.primary` | per container | Fill `--accent`, text `--t-bg`, weight 600, `--focus: var(--t-bg)`. Danger: `--danger-fill` + white. Bookmark: `--mark`. Make sure no container rule out-ranks it (test the computed fill). |
| Banner | `div[role=status]` under the header (`#status`, `#restored`, `#stall`, `#sized`) | style.css | Full width, 6px 12px, white on a fixed fill (§2.2); a close `.x` when it does not clear itself. |
| Toast | `#toast` via `toast(text, ms)` (app.js) | style.css | One line when short, wraps when long, never wider than the terminal column minus 32px; above the floating buttons on a phone. |
| Waiting card | `#asks .ask` > `.go` + `.x` | style.css | Starts under the banners and the find button (wide: `grid-row: 6 / -1`). |
| List row | `button.item` (`.pbar`, `.txt` > `.head` (`.idx`, `.agmark`, `.t`) + `.s`, `.st`) | style.css | Title ellipsis, meta line gives way from its end, under the title. Child rows `.item.child`. |
| Group header | `.grp h2` > `.grip`, `button.toggle`, `.attach`, `.import`, `.new`, `.hostbtn` | style.css, controls.css | The trailing buttons share size, radius and color (controls.css); `.new` is filled as the group's own action. The name keeps 6 characters; the session pill gives way first. |
| Group state | `p.gstate` under a group's name | tmux.css | One look for every state: a 7px blob dot in its severity (failed `--danger`, reconnecting `--waiting`, off and absent `--faint`), then terminumd's words in `--muted`, a failure's in `--danger-text`; a session of the user's tmux on a host that is down takes its host's state. |
| Badge / pill | `.tsess`, `.count`, `#scrlabel`, `#updchip`, `#marksfabn` | style.css, tmux.css, marks.css | Counts: muted 11px tabular; pills 999px. |
| Floating button | `#fabs > button`, `#findfab`, `#marksfab`, `#bottom` | style.css, find.css, marks.css | 44px on touch, blob radius, blurred `--chrome-2`; exclude from shapes.css. |
| Grip / splitter | `.grip` (rows), `[role=separator]` (`#navgrip`, `#quickgrip`) | style.css, tmux.css | A 6px band of the chrome between two surfaces (also docked Files' left edge, the Panel's `#splitter`), never a line; keyboard-operable (arrow keys), accent on hover/focus. |
| Window head | `.winhead` | style.css | Title 14px/600 left, `.icon` close right. |
| Output widget | `.wg` > `.bar` (name, `.seg` Raw/rendered, Copy) + `.body` (shadow root of a terminal row) | widgets.css | The design code inside the terminal too: the card is a `--surface-1` box with `--r-lg` and the wave, no border; the bar on the same surface; tables without frame or column lines, a muted head, rows 6×12px apart by a 7% hairline; Markdown headings apart by space, quotes on a tint, code on `--surface-0`; images on a quiet surface. The Panel's Markdown (`.md-body`) follows the same rules. |
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
- [ ] No new 1px border: space or a surface step instead (§2.1); a new box, field or header uses
      its shape class (§2.7); new motion uses the tokens, stays out of the terminal and has the
      reduced-motion switch (§2.8).
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
`web/test/e2e_tmux.mjs` (footer buttons share computed look, a phone's blobs differ and none is
round; form font sizes follow `--size`), `web/test/e2e_motion.mjs` (a popover comes in and goes
at once, a press ripples, the selected row settles, nothing moves in the terminal or with
reduced motion). A new kind of control adds its check there. The other e2e checks test
behavior, not look.
