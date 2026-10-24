// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The address a fetched file is shown from (AC-09), for the page and the Files panel. A blob:
// address has the page's origin and keeps no response header: one opened as a page (Open Image
// in New Tab, a video's Open in New Tab) runs its scripts there with the sign-in in reach, and
// Chromium applies none of the page's CSP to it. So only a picture, a video, a sound or a PDF (the
// browser's own viewer shows it, none of its script runs in the page's origin) keeps its type; an
// SVG, a picture that can hold script, is shown from a data: address (an origin of its own, never
// the page's); anything else is bytes, which a browser saves rather than opens.

const SHOWN = /^(image\/(png|jpeg|gif|webp|avif|bmp|apng|x-icon|vnd\.microsoft\.icon)|video\/[\w.+-]+|audio\/[\w.+-]+|application\/pdf)$/;

/** The type `type` (a Content-Type) may be shown as: itself, "svg" or "bytes". */
export function shownAs(type) {
  const t = String(type ?? "").split(";")[0].trim().toLowerCase();
  return t === "image/svg+xml" ? "svg" : SHOWN.test(t) ? t : "bytes";
}

async function svgUrl(blob) {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let text = "";
  for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return `data:image/svg+xml;base64,${btoa(text)}`;
}

/** An address to show `blob` from: blob: for a picture, a video, a sound or a PDF; data: for an SVG;
 *  blob: of bytes for anything else. The caller revokes it (nothing to revoke for data:). */
export async function shownUrl(blob) {
  const as = shownAs(blob.type);
  if (as === "svg") return svgUrl(blob);
  return URL.createObjectURL(as === "bytes" ? new Blob([blob], { type: "application/octet-stream" }) : blob);
}
