// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What a binary file's tab shows (AC-64): a picture; a PDF in the browser's own viewer; a video or
// a sound in the browser's player. A sound it does not play is decoded here (AU, AIFF: pcm.ts) or,
// a MIDI file, rendered by terminumd with the user's SoundFont (midi.rs) into a WAV for the same
// player; anything else is a card with Download (Open with default app on this Mac's own page).

import { esc, fmtSize, hydrateRaw, midiWav, PROXIED, rawBytes, SERVER_APPS, whileShown, type FileView } from "./api";
import { decoderFor, wav } from "./pcm";

let sounds = 0;            // the sounds decoded or rendered here, each named for whileShown
const isMidi = (f: FileView) => /^audio\/(x-)?midi$/.test(f.mime ?? "") || /\.(midi?|kar)$/i.test(f.path);

/** The view `f` gets by its type, or null for a plain binary card: a PDF may be all text. */
export function mediaOf(f: FileView): "image" | "pdf" | "video" | "audio" | null {
  if (f.mime?.startsWith("image/")) return "image";
  if (f.mime === "application/pdf") return "pdf";
  if (f.mime?.startsWith("video/")) return "video";
  return f.mime?.startsWith("audio/") || isMidi(f) ? "audio" : null;
}

/** Renders binary file `f` into `body` under `banner`. */
export function renderBinary(body: HTMLElement, f: FileView, banner: string) {
  const media = mediaOf(f), path = esc(f.path);
  const action = PROXIED || !SERVER_APPS ? `<button class="btn" data-download>Download</button>` // web app (AC-53), a Linux server
    : `<button class="btn" data-open-app>Open with default app</button>`;
  if (media === "image") {
    body.innerHTML = `${banner}<div class="scroll imgview"><img data-raw="${path}" alt="" title="Zoom"><div class="imgmeta"></div></div>`;
    // its own caption: the body may show another file by the time the picture loads
    const img = body.querySelector("img")!, meta = body.querySelector<HTMLElement>(".imgmeta")!;
    img.onload = () => { meta.textContent = `${img.naturalWidth} × ${img.naturalHeight} · ${fmtSize(f.size)}`; };
    return hydrateRaw(body);
  }
  // the address revoked once the tab closed (hydrateRaw, whileShown)
  if (media === "pdf") return void (body.innerHTML = `${banner}<div class="mediaview pdfview"><iframe data-raw="${path}" title="${path}"></iframe></div>`, hydrateRaw(body));
  if (media === "video") return void (body.innerHTML = `${banner}<div class="scroll mediaview"><video controls preload="metadata" data-raw="${path}"></video><div class="imgmeta">${fmtSize(f.size)}</div><div>${action}</div></div>`, hydrateRaw(body));
  if (media !== "audio") {
    body.innerHTML = `${banner}<div class="pinfo"><div class="big">Binary file</div>${esc(f.mime ?? "unknown type")} · ${fmtSize(f.size)}<div>${action}</div></div>`;
    return;
  }
  const decode = isMidi(f) ? null : decoderFor(f.path);
  const plays = !isMidi(f) && !!f.mime && document.createElement("audio").canPlayType(f.mime) !== "";
  if (!plays && !decode && !isMidi(f)) {
    body.innerHTML = `${banner}<div class="pinfo"><div class="big">This browser cannot play ${esc(f.mime ?? "this sound")}</div>${fmtSize(f.size)}<div>${action}</div></div>`;
    return;
  }
  body.innerHTML = `${banner}<div class="scroll mediaview"><audio controls preload="metadata"${plays ? ` data-raw="${path}"` : ""}></audio>` +
    `<div class="imgmeta">${fmtSize(f.size)}</div><div class="mediainfo"></div><div>${action}</div></div>`;
  if (plays) return hydrateRaw(body);
  // decoded or rendered here into a WAV the player plays
  const audio = body.querySelector("audio")!, info = body.querySelector<HTMLElement>(".mediainfo")!;
  info.textContent = isMidi(f) ? "Rendering the MIDI file…" : "Decoding…";
  // the tab closed or another file shown meanwhile: its render is called off, its WAV let go
  const wanted = new AbortController();
  let url = "";
  whileShown(`sound ${++sounds}`, audio, () => { wanted.abort(); if (url) URL.revokeObjectURL(url); });
  void (async () => {
    try {
      const bytes = await rawBytes(f.path);
      if (wanted.signal.aborted) return;
      if (isMidi(f)) {
        const got = await midiWav(bytes, wanted.signal);
        url = got.url;
        info.textContent = `MIDI · played with ${got.font || "the SoundFont"}`;
      } else {
        const pcm = decode!(bytes);
        url = URL.createObjectURL(new Blob([wav(pcm)], { type: "audio/wav" }));
        info.textContent = `${pcm.channels === 1 ? "Mono" : `${pcm.channels} channels`} · ${pcm.rate} Hz · decoded here`;
      }
      if (wanted.signal.aborted) return URL.revokeObjectURL(url);
      audio.src = url;
    } catch (e: any) {
      if (!wanted.signal.aborted) info.textContent = `Not played: ${e.message ?? e}`;
    }
  })();
}
