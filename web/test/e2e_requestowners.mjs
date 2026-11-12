// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-09, AC-33, AC-37: sign-in and dialogs keep their attempt after timeout, cancel and reopen.
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, pause, pressFab, SIZES } from "./fake.mjs";

export async function webRequestOwners(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, secret: "current-test-signin", auth: { login: "password" } });
    const { page } = f;
    await page.waitForSelector("#login:not([hidden])");
    await page.clock.install();
    const logins = [];
    await page.route("**/auth/login", (r) => logins.push(r));
    await page.fill("#password", "example password");
    await page.locator("#login").evaluate((el) => { el.requestSubmit(); el.requestSubmit(); });
    await until(() => logins.length > 0);
    check("AC-09 repeated submit during sign-in sends once and disables its button", logins.length === 1 && await page.locator("#login button").isDisabled());
    // A sign-out invalidates a response already in flight, including before its body arrives.
    await page.locator("#signout").evaluate((el) => el.click());
    await pause(page);
    await logins[0].fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ secret: "obsolete-test-signin" }) });
    await pause(page);
    check("AC-09 a late login after Sign out adopts no sign-in", await page.evaluate(() => localStorage.getItem("tm.session") === null && !document.getElementById("login").hidden));
    await page.fill("#password", "retry password");
    await page.locator("#login").evaluate((el) => el.requestSubmit());
    await until(() => logins.length > 1);
    await page.clock.fastForward(15001);
    check("AC-09 an unanswered sign-in releases its button with an inline timeout", !await page.locator("#login button").isDisabled() && (await page.textContent("#loginerr")).includes("15 s"));
    await logins[1].fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ secret: "expired-test-signin" }) }).catch(() => {});
    check("AC-09 request ownership raises no browser exception", f.errors.length === 0, f.errors.join(" | "));
    await f.close();

    const p = await openPage(browser, { device });
    await drawn(p.page);
    await p.page.clock.install();
    const last = (t) => p.sent.filter((m) => m.t === t).at(-1);
    const select = () => p.page.evaluate(() => {
      const row = document.querySelector("#screen .ln"), r = document.createRange();
      r.selectNodeContents(row); getSelection().removeAllRanges(); getSelection().addRange(r);
      document.dispatchEvent(new Event("selectionchange"));
    });
    await select();
    await pressFab(p.page, "#askfab");
    await p.page.fill("#askq", "first question");
    await p.page.locator('#askbox [value="add"]').click();
    await until(() => !!last("askno"));
    const oldAsk = last("askno");
    await p.page.clock.fastForward(10001);
    await p.page.fill("#askq", "second question");
    await p.page.locator('#askbox [value="add"]').click();
    await until(() => last("askno").ref !== oldAsk.ref);
    p.say({ t: "askno", ref: oldAsk.ref, n: 1 });
    await pause(p.page);
    check("AC-37 an expired quote-number reply cannot answer its retry", await p.page.locator("#askbox").evaluate((e) => e.open) && !p.sent.some((m) => (m.t === "paste" || m.t === "in")));
    p.say({ t: "askno", ref: last("askno").ref, n: 2 });
    check("AC-37 the matching retry adds the requested question once", await until(() => p.sent.some((m) => (m.t === "paste" || m.t === "in") && m.data.includes("second question") && m.data.includes("#0002"))));

    await select();
    await pressFab(p.page, "#markadd");
    await p.page.fill("#marknote", "first bookmark");
    await p.page.locator('#markbox [value="save"]').click();
    await until(() => !!last("mark"));
    const oldMark = last("mark");
    await p.page.locator('#markbox [value="cancel"]').click();
    await select();
    await pressFab(p.page, "#markadd");
    await p.page.fill("#marknote", "new bookmark");
    p.say({ t: "marked", ref: oldMark.ref, id: "old" });
    await pause(p.page);
    check("AC-33 a bookmark save cannot close or alter a reopened dialog", await p.page.locator("#markbox").evaluate((e) => e.open) && await p.page.inputValue("#marknote") === "new bookmark");
    await p.page.locator('#markbox [value="cancel"]').click();
    check("AC-33/37 dialog ownership raises no browser exceptions", p.errors.length === 0, p.errors.join(" | "));
    const copies = await p.page.evaluate(async () => {
      const { Input } = await import("./input.js");
      const writes = [], original = navigator.clipboard.writeText;
      navigator.clipboard.writeText = () => new Promise((resolve) => writes.push(resolve));
      const input = { options: { status() {} }, afterCopy() {} };
      try {
        const a = Input.prototype.copySelection.call(input, "earlier", true);
        const b = Input.prototype.copySelection.call(input, "later", true);
        writes[0](); await a;
        const held = input.keepSelection;
        writes[1](); await b;
        return held && !input.keepSelection;
      } finally { navigator.clipboard.writeText = original; }
    });
    check("AC-37 an earlier clipboard completion cannot release the later copy's selection hold", copies);
    await p.close();
  }
}
