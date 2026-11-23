// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The panel's preferences (hidden files, split, sort) sent to terminumd.

import { api, type Prefs } from "./api";

/** Preferences not yet saved (a failed PUT): the next change or reconnect sends them again. One
 *  PUT at a time, as the workspace's (saveNow), so terminumd keeps the newest. */
export class PrefsSaver {
  unsaved = false;
  private sending: Promise<void> = Promise.resolve();

  constructor(private prefs: () => Prefs, private failed: (error: string | null) => void) {}

  save() {
    this.unsaved = true;
    const send = () => this.send();
    this.sending = this.sending.then(send, send);
  }

  private async send() {
    if (!this.unsaved) return;   // an earlier PUT in the queue sent them already
    const sent = JSON.stringify(this.prefs());
    try {
      await api("PUT", "/api/prefs", { body: this.prefs() });
      this.failed(null);
      if (JSON.stringify(this.prefs()) === sent) this.unsaved = false;
    } catch (e) {
      this.failed(`Preferences not saved: ${(e as Error).message}`);
    }
  }
}
