// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Panel checks of files dropped from Finder on the tree (AC-95): the folder under the pointer is
// marked and named while the drag lasts (a file's row: its folder; a folder's row: that folder;
// below the rows: the root), the files are saved there, a name taken asks and Keep Both saves the
// next number, nothing written over. Run by e2e_panel.mjs on its page.
import fs from "node:fs";
import { check, SB, waitFor } from "./e2e_harness.mjs";

/** Dispatches `type` with files `names` (their text "hello <name>") on `sel`; then the marked rows and the line over the tree.
 *  The row is found and the event sent in one go, again while the panel did not take it: the tree
 *  draws its rows anew as a folder's listing comes, and an event sent to a row it just replaced
 *  reaches nothing (a busy machine makes that likelier). */
async function drag(page, sel, names, type) {
  for (let i = 0; i < 20; i++) {
    const taken = await page.evaluate(([sel, names, type]) => {
      const el = document.querySelector(sel);
      if (!el) return false;
      const dt = new DataTransfer();
      for (const n of names) dt.items.add(new File([`hello ${n}`], n, { type: "text/plain" }));
      const e = new DragEvent(type, { dataTransfer: dt, bubbles: true, cancelable: true });
      el.dispatchEvent(e);
      return e.defaultPrevented;                    // the tree took it (dragover and drop both say so)
    }, [sel, names, type]);
    if (taken) break;
    await page.waitForTimeout(100);
  }
  await page.waitForTimeout(150);
  return page.evaluate(() => {
    const note = document.querySelector(".dropnote");
    return { marked: [...document.querySelectorAll("#tree .row.drop")].map((r) => r.dataset.p), note: note && !note.hidden ? note.textContent : null };
  });
}

export async function panelDrop(page, { row }) {
  await page.click(row("data"));                         // open: its files' rows lie under it
  await page.waitForSelector(row("data/pkg.json"));
  const overFile = await drag(page, row("data/pkg.json"), ["a.txt", "b.txt"], "dragover");
  const overDir = await drag(page, row("docs"), ["a.txt"], "dragover");
  const overRoot = await drag(page, "#tree", ["a.txt"], "dragover");
  check("AC-95 files dragged over the tree: a file's row marks its folder and its rows, a folder's row that folder, below the rows the root; a line says where",
    overFile.marked.includes(`${SB}/data`) && overFile.marked.includes(`${SB}/data/pkg.json`) && !overFile.marked.includes(`${SB}/docs`) && overFile.note === "Upload 2 files to data"
      && JSON.stringify(overDir.marked) === JSON.stringify([`${SB}/docs`]) && overDir.note === "Upload 1 file to docs"
      && overRoot.note?.endsWith("(the root)") && overRoot.marked.includes(`${SB}/docs`), JSON.stringify({ overFile, overDir, overRoot }));

  await drag(page, row("data/pkg.json"), ["note.txt", "two.txt"], "drop");
  const saved = await waitFor(() => fs.existsSync(`${SB}/data/two.txt`)).then(() => true, () => false);
  const toast = await page.evaluate(() => document.getElementById("toast")?.textContent ?? "");
  const after = await page.evaluate(() => ({ marked: document.querySelectorAll("#tree .row.drop").length, note: !document.querySelector(".dropnote")?.hidden }));
  check("AC-95 dropped, each is saved in that folder with its bytes, the tree shows them, a toast says so; the marks go",
    saved && fs.readFileSync(`${SB}/data/note.txt`, "utf8") === "hello note.txt" && toast === "Uploaded 2 files to data" && !after.marked && !after.note
      && await page.waitForSelector(row("data/two.txt"), { timeout: 3000 }).then(() => true, () => false), JSON.stringify({ saved, toast, after }));

  fs.writeFileSync(`${SB}/data/note.txt`, "mine");
  await drag(page, row("data"), ["note.txt"], "drop");
  const asked = await page.waitForSelector(".modal .btn.primary", { timeout: 3000 }).then(() => page.$eval(".modal .mtitle", (e) => e.textContent), () => null);
  await page.click(".modal .btn.primary");
  const both = await waitFor(() => fs.existsSync(`${SB}/data/note 2.txt`)).then(() => true, () => false);
  check("AC-95 a name taken asks; Keep Both saves it as \"note 2.txt\", the file there kept as it was",
    asked === "“note.txt” is in data already" && both && fs.readFileSync(`${SB}/data/note.txt`, "utf8") === "mine" && fs.readFileSync(`${SB}/data/note 2.txt`, "utf8") === "hello note.txt",
    JSON.stringify({ asked, both }));
  for (const f of ["note.txt", "note 2.txt", "two.txt"]) fs.rmSync(`${SB}/data/${f}`);
  await page.click(row("data"));                         // folded again: the rows below stay in sight
}
