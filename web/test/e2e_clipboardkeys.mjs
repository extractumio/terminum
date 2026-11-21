// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-03: terminal clipboard shortcuts use the browser's platform and its own Settings switch.
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, spyCopies } from "./fake.mjs";

const COPY = "Control+Shift+KeyC", PASTE = "Control+Shift+KeyV";

export async function webClipboardKeys(browser, name) {
  for (const platform of ["Linux x86_64", "Win32", "MacIntel"]) {
    runningIn(`${name} ${platform}`);
    const f = await openPage(browser);
    const { page, sent } = f;
    try {
      await page.addInitScript((platform) => Object.defineProperty(navigator, "platform", { value: platform }), platform);
      await page.reload();
      await drawn(page);
      const keys = () => sent.filter((m) => m.t === "in").map((m) => m.data).join("");
      const press = (key) => page.keyboard.press(key);
      const sends = async (shortcuts, want) => {
        const before = keys().length;
        await page.focus("#kbd");
        for (const shortcut of shortcuts) await press(shortcut);
        return (await until(() => keys().length >= before + want.length)) && keys().slice(before) === want;
      };
      if (platform === "MacIntel") {
        check("AC-03 the shifted Ctrl keys go to the program by default on a Mac", await sends([PASTE, COPY], "\x16\x03"), JSON.stringify(keys()));
        continue;
      }

      await spyCopies(page);
      await page.evaluate(() => {
        document.activeElement.blur();
        const range = document.createRange();
        range.selectNodeContents(document.querySelector("#screen .ln"));
        getSelection().removeAllRanges();
        getSelection().addRange(range);
      });
      await press(COPY);
      check("AC-03 Ctrl+Shift+C copies the terminal selection with focus off the input", await page.evaluate(() => window.copied[0] === "$ ls"));
      const pasteKey = await page.evaluate(() => {
        getSelection().removeAllRanges();
        const key = new KeyboardEvent("keydown", { key: "V", code: "KeyV", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true });
        document.activeElement.dispatchEvent(key);
        return { prevented: key.defaultPrevented, focused: document.activeElement.id };
      });
      check("AC-03 Ctrl+Shift+V focuses the terminal input and leaves the browser's paste action available",
        pasteKey?.focused === "kbd" && !pasteKey.prevented, JSON.stringify(pasteKey));
      // Supply a browser paste event with fixture data without using the runner's clipboard.
      await page.evaluate(() => {
        const data = new DataTransfer();
        data.setData("text/plain", "paste probe");
        document.getElementById("kbd").dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
      });
      check("AC-03 the paste event sends the clipboard text through the terminal's paste path",
        await until(() => keys().includes("\x1b[200~paste probe\x1b[201~")), JSON.stringify(keys()));
      await press(COPY);
      check("AC-03 Ctrl+Shift+C without a selection leaves the clipboard alone", await page.evaluate(() => window.copied.length === 1));
      check("AC-03 unshifted Ctrl+V and Ctrl+C still go to the program",
        await sends(["Control+KeyV", "Control+KeyC"], "\x16\x03"), JSON.stringify(keys()));
      await press("Meta+KeyF");
      await page.evaluate(() => document.activeElement.dispatchEvent(new KeyboardEvent("keydown", { key: "V", ctrlKey: true, shiftKey: true, bubbles: true })));
      check("AC-03 Ctrl+Shift+V leaves the history search field focused", await page.evaluate(() => document.activeElement.id === "findq"));
      await press("Escape");

      const setting = page.locator("#shiftclip");
      check("AC-03 the clipboard switch defaults to on and handles no extra terminal bytes",
        await setting.isChecked() && keys() === "\x1b[200~paste probe\x1b[201~\x16\x03", JSON.stringify(keys()));
      await page.click("#menubtn");
      await setting.press("Space");
      await press("Escape");
      check("AC-03 turning the switch off immediately returns both keys to the program", await sends([PASTE, COPY], "\x16\x03"));
      await page.reload();
      await drawn(page);
      check("AC-03 the browser keeps the switch off across reloads", !(await setting.isChecked()) && await sends([PASTE, COPY], "\x16\x03"));
      check("AC-03 no page errors", !f.errors.length, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
