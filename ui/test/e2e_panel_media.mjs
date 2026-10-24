// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Panel checks of what View shows for a file that is not text (AC-64): a PDF in the browser's viewer,
// a video and sounds in its player (an AU decoded here, a MIDI file rendered by terminumd with a
// SoundFont); and of data files rendered (AC-94): JSON and YAML as a tree, CSV as a table.
// Run by e2e_panel.mjs on its page; `row(path)` is a tree row's selector.
import { check, SOUNDFONT } from "./e2e_harness.mjs";

export async function panelMedia(page, { row }) {
  // AC-64 a PDF in the browser's own viewer filling its tab, a video in the player: each from a blob: of its own type
  await page.evaluate(() => { window.cspSeen = []; document.addEventListener("securitypolicyviolation", (e) => window.cspSeen.push(`${e.violatedDirective} ${e.blockedURI}`)); });
  await page.click(row("docs/spec.pdf"));
  await page.waitForSelector('.pdfview iframe[src^="blob:"]', { timeout: 5000 }).catch(() => {});
  const pdf = await page.evaluate(() => { const f = document.querySelector(".pdfview iframe"), b = document.getElementById("vbody").getBoundingClientRect(), r = f?.getBoundingClientRect();
    return { src: f?.getAttribute("src")?.slice(0, 5), fill: r ? [Math.round(r.width - b.width), Math.round(b.bottom - r.bottom)] : null, binary: !!document.querySelector(".pinfo") }; });
  await page.click(row("docs/demo.mp4"));
  await page.waitForSelector('.mediaview video[src^="blob:"]', { timeout: 5000 }).catch(() => {});
  const video = await page.evaluate(() => { const v = document.querySelector(".mediaview video"); return { src: v?.getAttribute("src")?.slice(0, 5), controls: !!v?.controls, paused: v?.paused, action: !!document.querySelector(".mediaview [data-download], .mediaview [data-open-app]") }; });
  check("AC-64 a PDF opens in the browser's viewer filling its tab, a video in the player with its controls, not started, each from a blob:; no CSP refusal",
    pdf.src === "blob:" && pdf.fill && Math.abs(pdf.fill[0]) <= 2 && Math.abs(pdf.fill[1]) <= 2 && !pdf.binary && video.src === "blob:" && video.controls && video.paused && video.action
      && !(await page.evaluate(() => window.cspSeen.length)), JSON.stringify({ pdf, video, csp: await page.evaluate(() => window.cspSeen) }));

  // AC-64 sounds: an AU (Chrome plays none) decoded here into the player; a MIDI file rendered by terminumd
  await page.click(row("docs/beep.au"));
  await page.waitForFunction(() => document.querySelector(".mediaview audio")?.getAttribute("src")?.startsWith("blob:") || /Not played|cannot/.test(document.querySelector(".mediaview, .pinfo")?.textContent ?? ""), null, { timeout: 8000 }).catch(() => {});
  const au = await page.evaluate(() => ({ src: document.querySelector(".mediaview audio")?.getAttribute("src")?.slice(0, 5), info: document.querySelector(".mediainfo")?.textContent ?? "", controls: !!document.querySelector(".mediaview audio")?.controls }));
  await page.click(row("docs/groove.mid"));
  await page.waitForFunction(() => !/Rendering/.test(document.querySelector(".mediainfo")?.textContent ?? "Rendering"), null, { timeout: 20000 }).catch(() => {});
  const mid = await page.evaluate(() => ({ src: document.querySelector(".mediaview audio")?.getAttribute("src")?.slice(0, 5) ?? null, info: document.querySelector(".mediainfo")?.textContent ?? "" }));
  check(`AC-64 an AU plays in the browser's player (decoded here where it does not play it); a MIDI file ${SOUNDFONT ? "plays rendered with the SoundFont" : "with no SoundFont says where to put one"}`,
    au.src === "blob:" && au.controls && (SOUNDFONT ? mid.src === "blob:" && mid.info.includes("soundfont.sf2") : !mid.src && /No SoundFont for MIDI: put a General MIDI \.sf2 at .*soundfont\.sf2/.test(mid.info)), JSON.stringify({ au, mid }));

  // AC-94 JSON and YAML as a tree in the editor's colors, open; CSV as a table; an error said where
  await page.click(row("data"));
  await page.click(row("data/pkg.json"));
  await page.waitForSelector(".dataview .dr", { timeout: 5000 }).catch(() => {});
  const json = await page.evaluate(() => ({ keys: [...document.querySelectorAll(".dataview .dr .tok-propertyName")].map((e) => e.textContent),
    types: ["tok-string", "tok-number", "tok-bool", "tok-atom"].map((c) => !!document.querySelector(`.dataview .${c}`)), seg: document.querySelector('#tools [data-act="rendered"]')?.classList.contains("on") }));
  await page.click('.dataview .dr:has(.tok-propertyName:text-is("deps"))');
  const folded = await page.evaluate(() => [...document.querySelectorAll(".dataview .dr .tok-propertyName")].filter((e) => e.offsetParent).map((e) => e.textContent));
  check("AC-94 JSON opens Rendered as a tree, open to its depths (small), keys and values in the editor's colors; a click folds a branch",
    json.seg && ["name", "deps", "a", "b", "c", "0", "list"].every((k) => json.keys.includes(k)) && json.types.every(Boolean) && !folded.includes("c") && folded.includes("list"), JSON.stringify({ json, folded }));
  await page.click(row("data/conf.yaml"));
  await page.waitForFunction(() => document.querySelectorAll(".dataview .ddoc").length === 2, null, { timeout: 5000 }).catch(() => {});
  const yaml = await page.evaluate(() => ({ docs: document.querySelectorAll(".dataview .ddoc").length, port: [...document.querySelectorAll(".dataview .tok-number")].map((e) => e.textContent) }));
  await page.click(row("data/table.csv"));
  await page.waitForSelector(".dtab", { timeout: 5000 }).catch(() => {});
  const csv = await page.evaluate(() => ({ head: [...document.querySelectorAll(".dtab thead th")].map((e) => e.textContent), cells: [...document.querySelectorAll(".dtab tbody tr:first-child td")].map((e) => e.textContent),
    num: getComputedStyle(document.querySelector(".dtab tbody td.num") ?? document.body).textAlign }));
  await page.click(row("data/bad.json"));
  await page.waitForSelector(".dataview .dataerr", { timeout: 5000 }).catch(() => {});
  const bad = await page.evaluate(() => document.querySelector(".dataview .dataerr")?.textContent ?? "");
  await page.click('#tools [data-act="source"]');
  const source = await page.waitForSelector(".cm-content", { timeout: 3000 }).then(() => true, () => false);
  check("AC-94 YAML's two documents, CSV as a table (its head, a quoted comma one cell, numbers right), a bad JSON says the line and column; Source shows the text",
    yaml.docs === 2 && yaml.port.includes("8080") && JSON.stringify(csv.head) === JSON.stringify(["#", "name", "count", "note"]) && csv.cells[3] === "with, comma" && csv.num === "right"
      && /^Line 3, column 8: /.test(bad) && source, JSON.stringify({ yaml, csv, bad, source }));
  await page.click(row("data"));                       // folded again, docs too: the rows below stay in sight
  await page.click(row("docs"));
}
