// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AU and AIFF decoded into a WAV a browser plays (pcm.ts, AC-64).
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";
const { decodeAu, decodeAiff, decoderFor, wav } = await load("src/pcm.ts");

function au(encoding, data, rate = 8000, channels = 1) {
  const b = Buffer.alloc(24 + data.length);
  b.write(".snd", 0); b.writeUInt32BE(24, 4); b.writeUInt32BE(data.length, 8); b.writeUInt32BE(encoding, 12); b.writeUInt32BE(rate, 16); b.writeUInt32BE(channels, 20);
  Buffer.from(data).copy(b, 24);
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.length);
}

test("AU: μ-law, 16-bit PCM and A-law; an unknown encoding or no AU said", () => {
  const mu = decodeAu(au(1, [0xff, 0x7f, 0x80, 0x00]));
  assert.deepEqual([mu.rate, mu.channels, mu.samples.length], [8000, 1, 4]);
  assert.ok(Math.abs(mu.samples[0]) < 0.001 && mu.samples[2] > 0.9 && mu.samples[3] < -0.9, String([...mu.samples]));
  const pcm = decodeAu(au(3, [0x40, 0x00, 0xc0, 0x00], 44100, 2));
  assert.deepEqual([pcm.rate, pcm.channels, ...pcm.samples], [44100, 2, 0.5, -0.5]);
  assert.ok(decodeAu(au(27, [0xd5])).samples.length === 1);
  assert.throws(() => decodeAu(au(23, [0])), /encoding 23/);
  assert.throws(() => decodeAu(new ArrayBuffer(30)), /not an AU/);
});

test("AIFF: 16-bit big-endian PCM at its 80-bit rate", () => {
  const ssnd = [0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0x00, 0xc0, 0x00];
  const comm = [0, 1, 0, 0, 0, 2, 0, 16, 0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0];      // 1 channel, 2 frames, 16 bits, 44100 Hz
  const chunk = (id, body) => [...Buffer.from(id), 0, 0, 0, body.length, ...body];
  const body = [...Buffer.from("AIFF"), ...chunk("COMM", comm), ...chunk("SSND", ssnd)];
  const file = Buffer.from([...Buffer.from("FORM"), 0, 0, 0, body.length, ...body]);
  const a = decodeAiff(file.buffer.slice(file.byteOffset, file.byteOffset + file.length));
  assert.deepEqual([a.rate, a.channels, ...a.samples], [44100, 1, 0.5, -0.5]);
});

test("a WAV of 16-bit PCM; the decoder by the file's name", () => {
  const w = new DataView(wav({ rate: 8000, channels: 1, samples: new Float32Array([1, -1]) }));
  assert.equal(String.fromCharCode(w.getUint8(0), w.getUint8(1), w.getUint8(2), w.getUint8(3)), "RIFF");
  assert.deepEqual([w.getUint32(24, true), w.getInt16(44, true), w.getInt16(46, true)], [8000, 32767, -32767]);
  assert.equal(decoderFor("x.au"), decodeAu);
  assert.equal(decoderFor("x.AIFF"), decodeAiff);
  assert.equal(decoderFor("x.mp3"), null);
});
