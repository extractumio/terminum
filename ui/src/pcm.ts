// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AU (.au, .snd) and AIFF (.aif, .aiff, AIFF-C) decoded into a WAV a browser plays (AC-64): Chrome
// plays neither. Pure: no DOM. Samples become 16-bit PCM; channels and rate are kept.

export interface Pcm { rate: number; channels: number; samples: Float32Array }   // interleaved, -1…1

const ulaw = (b: number) => { b = ~b & 255; const t = (((b & 15) << 3) + 0x84) << ((b & 0x70) >> 4); return (b & 128 ? 0x84 - t : t - 0x84) / 32768; };
const alaw = (b: number) => { b ^= 0x55; const e = (b & 0x70) >> 4; let t = (b & 15) << 4; t = e ? (t + 0x108) << (e - 1) : t + 8; return (b & 128 ? t : -t) / 32768; };

/** Reads `n` samples of `encoding` from `view` at `at`: "u8" μ-law, "a8" A-law, "s8"/"s16"/"s24"/"s32" PCM, "f32"/"f64" floats. */
function read(view: DataView, at: number, n: number, encoding: string, little = false): Float32Array {
  const size = { u8: 1, a8: 1, s8: 1, s16: 2, s24: 3, s32: 4, f32: 4, f64: 8 }[encoding]!;
  const out = new Float32Array(Math.max(0, Math.min(n, Math.floor((view.byteLength - at) / size))));
  for (let i = 0, p = at; i < out.length; i++, p += size) {
    switch (encoding) {
      case "u8": out[i] = ulaw(view.getUint8(p)); break;
      case "a8": out[i] = alaw(view.getUint8(p)); break;
      case "s8": out[i] = view.getInt8(p) / 128; break;
      case "s16": out[i] = view.getInt16(p, little) / 32768; break;
      case "s24": { const v = little ? view.getUint8(p) | (view.getUint8(p + 1) << 8) | (view.getInt8(p + 2) << 16) : (view.getInt8(p) << 16) | (view.getUint8(p + 1) << 8) | view.getUint8(p + 2); out[i] = v / 8388608; break; }
      case "s32": out[i] = view.getInt32(p, little) / 2147483648; break;
      case "f32": out[i] = view.getFloat32(p, little); break;
      case "f64": out[i] = view.getFloat64(p, little); break;
    }
  }
  return out;
}

const tag = (v: DataView, at: number) => String.fromCharCode(v.getUint8(at), v.getUint8(at + 1), v.getUint8(at + 2), v.getUint8(at + 3));

/** An AU file's samples; throws why not. */
export function decodeAu(buf: ArrayBuffer): Pcm {
  const v = new DataView(buf);
  if (v.byteLength < 24 || tag(v, 0) !== ".snd") throw new Error("not an AU file");
  const offset = v.getUint32(4), size = v.getUint32(8), code = v.getUint32(12), rate = v.getUint32(16), channels = v.getUint32(20);
  const encoding = ({ 1: "u8", 2: "s8", 3: "s16", 4: "s24", 5: "s32", 6: "f32", 7: "f64", 27: "a8" } as Record<number, string>)[code];
  if (!encoding) throw new Error(`AU encoding ${code} is not one this panel decodes`);
  if (!rate || !channels || channels > 32) throw new Error("its rate or channels are not readable");
  const bytes = size === 0xffffffff ? v.byteLength - offset : Math.min(size, v.byteLength - offset);
  const per = { u8: 1, a8: 1, s8: 1, s16: 2, s24: 3, s32: 4, f32: 4, f64: 8 }[encoding]!;
  return { rate, channels, samples: read(v, offset, Math.floor(bytes / per), encoding) };
}

/** An 80-bit IEEE extended float (AIFF's sample rate). */
function extended(v: DataView, at: number): number {
  const exp = v.getUint16(at) & 0x7fff, hi = v.getUint32(at + 2), lo = v.getUint32(at + 6);
  return exp === 0 && hi === 0 && lo === 0 ? 0 : (hi * 2 ** 32 + lo) * 2 ** (exp - 16383 - 63);
}

/** An AIFF or AIFF-C file's samples; throws why not. */
export function decodeAiff(buf: ArrayBuffer): Pcm {
  const v = new DataView(buf);
  if (v.byteLength < 12 || tag(v, 0) !== "FORM" || !/^AIF[FC]$/.test(tag(v, 8))) throw new Error("not an AIFF file");
  let channels = 0, frames = 0, bits = 0, rate = 0, kind = "NONE", data = -1;
  for (let at = 12; at + 8 <= v.byteLength;) {
    const id = tag(v, at), len = v.getUint32(at + 4), body = at + 8;
    if (id === "COMM") {
      channels = v.getInt16(body); frames = v.getUint32(body + 2); bits = v.getInt16(body + 6); rate = extended(v, body + 8);
      if (tag(v, 8) === "AIFC" && len >= 22) kind = tag(v, body + 18);
    } else if (id === "SSND") data = body + 8 + v.getUint32(body);
    at = body + len + (len & 1);
  }
  if (!channels || !rate || data < 0) throw new Error("its COMM or SSND chunk is missing");
  const little = kind === "sowt";
  const encoding = kind === "fl32" || kind === "FL32" ? "f32" : kind === "fl64" || kind === "FL64" ? "f64" : kind === "ulaw" ? "u8" : kind === "alaw" ? "a8"
    : /^(NONE|twos|sowt)$/.test(kind) ? `s${Math.ceil(bits / 8) * 8}` : "";
  if (!["s8", "s16", "s24", "s32", "f32", "f64", "u8", "a8"].includes(encoding)) throw new Error(`AIFF-C compression ${kind} is not one this panel decodes`);
  return { rate: Math.round(rate), channels, samples: read(v, data, frames * channels, encoding, little) };
}

/** A 16-bit PCM WAV of `pcm`. */
export function wav({ rate, channels, samples }: Pcm): ArrayBuffer {
  const out = new ArrayBuffer(44 + samples.length * 2), v = new DataView(out);
  const put = (at: number, s: string) => { for (let i = 0; i < 4; i++) v.setUint8(at + i, s.charCodeAt(i)); };
  put(0, "RIFF"); v.setUint32(4, 36 + samples.length * 2, true); put(8, "WAVE");
  put(12, "fmt "); v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, channels, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate * channels * 2, true); v.setUint16(32, channels * 2, true); v.setUint16(34, 16, true);
  put(36, "data"); v.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i++) v.setInt16(44 + i * 2, Math.max(-1, Math.min(1, samples[i])) * 32767, true);
  return out;
}

/** The decoder for a file name, or null: AU and AIFF. */
export function decoderFor(path: string): ((b: ArrayBuffer) => Pcm) | null {
  return /\.(au|snd)$/i.test(path) ? decodeAu : /\.(aiff?|aifc)$/i.test(path) ? decodeAiff : null;
}
