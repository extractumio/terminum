# Reuse map: iterm-enhancer 0.42 → terminum

What each source file of iterm-enhancer becomes. Status: **keep** = copied as it is; **adapt** = the same file with small edits (named); **port** = the same logic written in Rust (Python today); **rewrite** = the same purpose, a new design for tmux; **drop** = no successor (iTerm2 only). Tests follow their modules: `bridge/tests/test_web_*.py` for ported modules become `cargo test` cases; `ui/test/*` and `web/test` e2e checks are kept and run against terminumd.

| Status | Lines |
|---|---|
| keep | 10335 |
| adapt | 5470 |
| port | 3390 |
| rewrite | 1898 |
| drop | 7180 |
| total | 28273 |

## terminumd ← fbd/src (Rust)

Lines: keep 2517, adapt 1409, rewrite 597, drop 1965

| File | Lines | Status | Note |
|---|---|---|---|
| `fbd/src/listing.rs` | 436 | keep | listing cache, paging, filter |
| `fbd/src/ops.rs` | 369 | keep | save, mkdir, touch, rename, trash |
| `fbd/src/workspace.rs` | 298 | keep | workspaces, TTL |
| `fbd/src/remote.rs` | 287 | keep | agent proxy |
| `fbd/src/api_fs.rs` | 228 | keep | the /fb/api routes |
| `fbd/src/files.rs` | 215 | keep | reads, etag, limits |
| `fbd/src/agent.rs` | 162 | keep | agent mode |
| `fbd/src/termtext.rs` | 120 | keep | quoting for the pane's shell |
| `fbd/src/events.rs` | 108 | keep | event bus |
| `fbd/src/watcher.rs` | 100 | keep | FSEvents |
| `fbd/src/ops_tests.rs` | 100 | keep |  |
| `fbd/src/remote_tests.rs` | 94 | keep |  |
| `fbd/src/main.rs` | 454 | adapt | wiring of the new modules |
| `fbd/src/local.rs` | 200 | adapt | local or remote routing by X-TM-Host |
| `fbd/src/socket.rs` | 185 | adapt | the CLI socket: health, codes, web switches |
| `fbd/src/watch_web.rs` | 150 | adapt | web: keys become the only keys |
| `fbd/src/web.rs` | 114 | adapt | web.json and the web listener's refusals |
| `fbd/src/auth.rs` | 105 | adapt | sign-in header and subprotocol, the hello proof; Host, Origin, Content-Type checks stay |
| `fbd/src/update.rs` | 103 | adapt | release check: tag parsing stays, the daily request moves in from updates.py |
| `fbd/src/http.rs` | 98 | adapt | two listeners, CSP |
| `fbd/src/api_state.rs` | 435 | rewrite | state per shown pane from the layout, no focused pane, no window claims |
| `fbd/src/liveness.rs` | 162 | rewrite | inventory from list-panes |
| `fbd/src/recovery_store.rs` | 374 | drop | native checkpoints |
| `fbd/src/panels.rs` | 314 | drop | panel-per-window claims |
| `fbd/src/checkpoint.rs` | 261 | drop |  |
| `fbd/src/recovery_startup_tests.rs` | 232 | drop |  |
| `fbd/src/recovery.rs` | 179 | drop |  |
| `fbd/src/recovery_quit_tests.rs` | 155 | drop |  |
| `fbd/src/recovery_lifecycle_tests.rs` | 144 | drop |  |
| `fbd/src/recovery_tests.rs` | 120 | drop |  |
| `fbd/src/recovery_lifecycle.rs` | 100 | drop |  |
| `fbd/src/recovery_startup.rs` | 86 | drop |  |

## terminumd ← bridge/fbbridge/web (Python → Rust)

Lines: port 2106, rewrite 1208, drop 174

| File | Lines | Status | Note |
|---|---|---|---|
| `bridge/fbbridge/web/trace.py` | 328 | port | ?trace=1 records; stage names change |
| `bridge/fbbridge/web/bookmarks.py` | 325 | port | bookmarks store and locate |
| `bridge/fbbridge/web/httpd.py` | 282 | port | limits before sign-in; axum instead of a hand-written server |
| `bridge/fbbridge/web/fold.py` | 248 | port | folds: pure functions of lines |
| `bridge/fbbridge/web/site.py` | 188 | port | static files, build id, no-cache |
| `bridge/fbbridge/web/access.py` | 143 | port | sign-ins file, Host allowlist |
| `bridge/fbbridge/web/auth.py` | 130 | port | password, PBKDF2, brakes |
| `bridge/fbbridge/web/paste.py` | 103 | port | local save as is; a host through the agent |
| `bridge/fbbridge/web/tmuxkeys.py` | 75 | port | key names table; now for every pane |
| `bridge/fbbridge/web/config.py` | 70 | port | web.json |
| `bridge/fbbridge/web/search.py` | 65 | port | history search |
| `bridge/fbbridge/web/rename.py` | 64 | port | rename-window, select-pane -T, with ## escaping |
| `bridge/fbbridge/web/agentstate.py` | 57 | port | the agent's state from 25 rows |
| `bridge/fbbridge/web/net.py` | 28 | port |  |
| `bridge/fbbridge/web/mirror.py` | 497 | rewrite | the frame loop on %output and capture-pane |
| `bridge/fbbridge/web/newsession.py` | 216 | rewrite | new-window, new-session, split-window |
| `bridge/fbbridge/web/layout.py` | 184 | rewrite | groups are tmux sessions; title rules kept |
| `bridge/fbbridge/web/hub.py` | 130 | rewrite | layout refresh from list-panes and notices |
| `bridge/fbbridge/web/screen.py` | 74 | rewrite | the same line format, from capture-pane and SGR instead of iTerm2 cells |
| `bridge/fbbridge/web/merge.py` | 63 | rewrite | move-window |
| `bridge/fbbridge/web/stall.py` | 44 | rewrite | a control client that does not answer |
| `bridge/fbbridge/web/proxy.py` | 174 | drop | no proxy: one process |

## terminumd ← bridge/fbbridge (Python → Rust)

Lines: port 1284, drop 3212

| File | Lines | Status | Note |
|---|---|---|---|
| `bridge/fbbridge/recovery_tmux.py` | 329 | port | only remap_layout and the layout-string handling for AC-15 (about 40 lines); the rest drops |
| `bridge/fbbridge/agentctl.py` | 284 | port | enable a host's files, copy the helper |
| `bridge/fbbridge/remote.py` | 251 | port | agent tunnel, back-off, helper updates |
| `bridge/fbbridge/recovery_recipes.py` | 139 | port | only ssh_recipe (50 lines); the rest drops |
| `bridge/fbbridge/updates.py` | 112 | port | daily release check |
| `bridge/fbbridge/hosts.py` | 74 | port | hosts records (settings.json) |
| `bridge/fbbridge/sshargs.py` | 54 | port | ssh recipe allowlist, without -F |
| `bridge/fbbridge/common.py` | 41 | port |  |
| `bridge/fbbridge/app.py` | 330 | drop | iTerm2 app loop |
| `bridge/fbbridge/recovery_native.py` | 308 | drop |  |
| `bridge/fbbridge/recovery_lifecycle.py` | 263 | drop |  |
| `bridge/fbbridge/recovery_capture.py` | 252 | drop |  |
| `bridge/fbbridge/tmuxwatch.py` | 208 | drop | no dropped -CC |
| `bridge/fbbridge/windows.py` | 204 | drop |  |
| `bridge/fbbridge/procinfo.py` | 164 | drop | no process sniffing |
| `bridge/fbbridge/resolve.py` | 158 | drop | cwd from pane_current_path |
| `bridge/fbbridge/recovery_tmux_capture.py` | 143 | drop |  |
| `bridge/fbbridge/settings.py` | 125 | drop | iTerm2 preferences |
| `bridge/fbbridge/recovery_runner.py` | 125 | drop |  |
| `bridge/fbbridge/backend.py` | 120 | drop | fbd supervision: launchd |
| `bridge/fbbridge/lifecycle.py` | 117 | drop |  |
| `bridge/fbbridge/recovery_startup.py` | 84 | drop |  |
| `bridge/fbbridge/rpcstats.py` | 74 | drop |  |
| `bridge/fbbridge/inventory.py` | 71 | drop |  |
| `bridge/fbbridge/recovery_quit.py` | 68 | drop |  |
| `bridge/fbbridge/recovery_exit.py` | 67 | drop |  |
| `bridge/fbbridge/registry.py` | 64 | drop |  |
| `bridge/fbbridge/setup_state.py` | 63 | drop |  |
| `bridge/fbbridge/profiler.py` | 54 | drop |  |
| `bridge/fbbridge/viewer_profile.py` | 45 | drop |  |
| `bridge/fbbridge/unixhttp.py` | 33 | drop |  |
| `bridge/fbbridge/recovery_liveness.py` | 30 | drop |  |
| `bridge/fb_bridge.py` | 26 | drop | AutoLaunch entry |
| `bridge/fbbridge/navigation.py` | 16 | drop |  |

## web/static (page, copied)

Lines: keep 4737, adapt 2136

| File | Lines | Status | Note |
|---|---|---|---|
| `bridge/fbbridge/web/static/style.css` | 451 | keep |  |
| `bridge/fbbridge/web/static/widgets.js` | 358 | keep |  |
| `bridge/fbbridge/web/static/tracebuf.js` | 325 | keep |  |
| `bridge/fbbridge/web/static/marks.js` | 279 | keep |  |
| `bridge/fbbridge/web/static/outparse.js` | 229 | keep |  |
| `bridge/fbbridge/web/static/markjump.js` | 215 | keep |  |
| `bridge/fbbridge/web/static/widgetui.js` | 206 | keep |  |
| `bridge/fbbridge/web/static/trace.js` | 203 | keep |  |
| `bridge/fbbridge/web/static/boxnav.js` | 177 | keep |  |
| `bridge/fbbridge/web/static/folds.js` | 166 | keep |  |
| `bridge/fbbridge/web/static/traceshot.js` | 162 | keep |  |
| `bridge/fbbridge/web/static/find.js` | 150 | keep |  |
| `bridge/fbbridge/web/static/panefiles.js` | 136 | keep |  |
| `bridge/fbbridge/web/static/zoom.js` | 128 | keep |  |
| `bridge/fbbridge/web/static/markpick.js` | 125 | keep |  |
| `bridge/fbbridge/web/static/render.js` | 115 | keep |  |
| `bridge/fbbridge/web/static/cursor.js` | 105 | keep |  |
| `bridge/fbbridge/web/static/marktext.js` | 100 | keep |  |
| `bridge/fbbridge/web/static/widgets.css` | 96 | keep |  |
| `bridge/fbbridge/web/static/marks.css` | 94 | keep |  |
| `bridge/fbbridge/web/static/termlinks.js` | 94 | keep |  |
| `bridge/fbbridge/web/static/waiting.js` | 92 | keep |  |
| `bridge/fbbridge/web/static/links.js` | 82 | keep |  |
| `bridge/fbbridge/web/static/agentbox.js` | 74 | keep |  |
| `bridge/fbbridge/web/static/linkmarks.js` | 74 | keep |  |
| `bridge/fbbridge/web/static/icons.js` | 69 | keep |  |
| `bridge/fbbridge/web/static/upload.js` | 64 | keep |  |
| `bridge/fbbridge/web/static/touchcopy.js` | 61 | keep |  |
| `bridge/fbbridge/web/static/shapes.css` | 60 | keep |  |
| `bridge/fbbridge/web/static/rejoin.js` | 59 | keep |  |
| `bridge/fbbridge/web/static/reorder.js` | 55 | keep |  |
| `bridge/fbbridge/web/static/find.css` | 38 | keep |  |
| `bridge/fbbridge/web/static/seltext.js` | 37 | keep |  |
| `bridge/fbbridge/web/static/rowtext.js` | 32 | keep |  |
| `bridge/fbbridge/web/static/heldelete.js` | 26 | keep |  |
| `bridge/fbbridge/web/static/app.js` | 500 | adapt | message renames (stall, settings, restore), sign-in header |
| `bridge/fbbridge/web/static/term.js` | 457 | adapt | Screen mode, size message |
| `bridge/fbbridge/web/static/input.js` | 398 | adapt | mouse in Screen mode |
| `bridge/fbbridge/web/static/index.html` | 221 | adapt | merge box, menu without iTerm items |
| `bridge/fbbridge/web/static/nav.js` | 182 | adapt | group is a tmux session |
| `bridge/fbbridge/web/static/views.js` | 162 | adapt | /view tab |
| `bridge/fbbridge/web/static/navtools.js` | 155 | adapt | newsession, move, split, kill |
| `bridge/fbbridge/web/static/keys.js` | 61 | adapt | hotkeys.json |

## ui/src (panel, copied)

Lines: keep 2685, adapt 732, drop 205

| File | Lines | Status | Note |
|---|---|---|---|
| `ui/src/tree.ts` | 481 | keep |  |
| `ui/src/viewer.ts` | 480 | keep |  |
| `ui/src/style.css` | 278 | keep |  |
| `ui/src/docview.ts` | 108 | keep |  |
| `ui/src/icons.ts` | 98 | keep |  |
| `ui/src/tree-edit.ts` | 98 | keep |  |
| `ui/src/tree-rows.ts` | 97 | keep |  |
| `ui/src/dialogs.ts` | 84 | keep |  |
| `ui/src/tree-expand.ts` | 79 | keep |  |
| `ui/src/binding.ts` | 69 | keep |  |
| `ui/src/context-menu.ts` | 67 | keep |  |
| `ui/src/upgrade.ts` | 65 | keep |  |
| `ui/src/sha256.ts` | 64 | keep |  |
| `ui/src/stream.ts` | 64 | keep |  |
| `ui/src/file-actions.ts` | 62 | keep |  |
| `ui/src/highlight.ts` | 62 | keep |  |
| `ui/src/embed.ts` | 58 | keep |  |
| `ui/src/host-offer.ts` | 54 | keep |  |
| `ui/src/viewer-editor.ts` | 48 | keep |  |
| `ui/src/tree-render.ts` | 45 | keep |  |
| `ui/src/markdown-core.ts` | 42 | keep |  |
| `ui/src/web-widgets.ts` | 37 | keep |  |
| `ui/src/theme.ts` | 33 | keep |  |
| `ui/src/paths.ts` | 29 | keep |  |
| `ui/src/markdown.ts` | 26 | keep |  |
| `ui/src/viewer-path.ts` | 25 | keep |  |
| `ui/src/tree-select.ts` | 15 | keep |  |
| `ui/src/panel-refresh.ts` | 10 | keep |  |
| `ui/src/shims.d.ts` | 7 | keep |  |
| `ui/src/main.ts` | 470 | adapt | no window binding; panel always pinned |
| `ui/src/api.ts` | 185 | adapt | sign-in header instead of ?t= |
| `ui/src/iterm-tools.ts` | 77 | adapt | update chip stays; session nav and recovery buttons go |
| `ui/src/recovery.ts` | 104 | drop | native recovery dialog |
| `ui/src/recovery-list.ts` | 61 | drop |  |
| `ui/src/web-access.ts` | 40 | drop | web access is the CLI's |

## scripts

Lines: keep 396, adapt 1193, rewrite 93, drop 1624

| File | Lines | Status | Note |
|---|---|---|---|
| `scripts/release.py` | 157 | keep |  |
| `scripts/agents.py` | 147 | keep |  |
| `scripts/signing_key.py` | 52 | keep |  |
| `scripts/bench_ls.sh` | 25 | keep |  |
| `scripts/make_big_dir.sh` | 15 | keep |  |
| `scripts/install.py` | 321 | port | Rust in terminumd (`src/cli/install.rs`): launchd agent with socket activation instead of AutoLaunch |
| `scripts/cli.py` | 274 | port | Rust in terminumd (`src/cli/`): the `terminum` command is terminumd run by that name |
| `scripts/trace_report.py` | 169 | adapt | stage names |
| `scripts/e2e_remote.py` | 152 | adapt | hosts over ssh to localhost |
| `scripts/package.py` | 103 | adapt | no bridge in the package |
| `scripts/install.sh` | 62 | adapt | names |
| `scripts/e2e_common.py` | 58 | adapt |  |
| `scripts/security_check.sh` | 54 | adapt |  |
| `scripts/install_launch.py` | 93 | port | Rust in terminumd (`src/cli/launch.rs`): launchctl, health over the socket |
| `scripts/e2e_recovery.py` | 375 | drop | iTerm2 |
| `scripts/e2e_isolated.py` | 161 | drop |  |
| `scripts/e2e_connection_failure.py` | 157 | drop |  |
| `scripts/e2e_web_new.py` | 150 | drop | becomes web/test/e2e_sessions.mjs |
| `scripts/e2e_cwd.py` | 145 | drop | becomes web/test/e2e_files.mjs |
| `scripts/e2e_windows.py` | 134 | drop |  |
| `scripts/e2e_restore_platform.py` | 118 | drop |  |
| `scripts/e2e_terminal.py` | 106 | drop |  |
| `scripts/e2e_web_merge.py` | 102 | drop |  |
| `scripts/e2e_web_bookmarks.py` | 72 | drop | the mjs check covers it |
| `scripts/e2e_web_rename.py` | 63 | drop |  |
| `scripts/clean_registrations.py` | 41 | drop |  |
