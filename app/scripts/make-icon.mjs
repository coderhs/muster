// Renders icons without image tooling.
//   node scripts/make-icon.mjs out.png          app icon source, 1024x1024 (then `npx tauri icon`)
//   node scripts/make-icon.mjs --tray out.png   menu-bar template icon, 44x44, black on clear
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const TRAY = process.argv[2] === "--tray";
const OUT = TRAY ? process.argv[3] : process.argv[2];
const S = TRAY ? 44 : 1024;
const px = Buffer.alloc(S * S * 4);
const inRound = (x, y, x0, y0, x1, y1, r) => {
  if (x < x0 || x > x1 || y < y0 || y > y1) return 0;
  const cx = Math.min(Math.max(x, x0 + r), x1 - r);
  const cy = Math.min(Math.max(y, y0 + r), y1 - r);
  const d = Math.hypot(x - cx, y - cy);
  return Math.max(0, Math.min(1, r - d + 0.5));
};
const blend = (i, [r, g, b], a) => {
  const ia = 1 - a;
  px[i] = px[i] * ia + r * a;
  px[i + 1] = px[i + 1] * ia + g * a;
  px[i + 2] = px[i + 2] * ia + b * a;
  px[i + 3] = Math.max(px[i + 3], a * 255);
};
const cols = [
  { x: 232, h: 520, c: [120, 132, 150] },
  { x: 436, h: 380, c: [74, 144, 226] },
  { x: 640, h: 260, c: [82, 196, 120] },
];
const trayCols = [
  { x: 6, h: 32 },
  { x: 18, h: 24 },
  { x: 30, h: 16 },
];
for (let y = 0; y < S && TRAY; y++) {
  for (let x = 0; x < S; x++) {
    const i = (y * S + x) * 4;
    for (const c of trayCols) {
      const a = inRound(x, y, c.x, 6, c.x + 8, 6 + c.h, 2.5);
      if (a > 0) blend(i, [0, 0, 0], a);
    }
  }
}
for (let y = 0; y < S && !TRAY; y++) {
  for (let x = 0; x < S; x++) {
    const i = (y * S + x) * 4;
    const a = inRound(x, y, 100, 100, 924, 924, 190);
    if (a > 0) {
      const t = y / S;
      blend(i, [34 - 10 * t, 37 - 10 * t, 46 - 10 * t], a);
    }
    for (const c of cols) {
      const ca = inRound(x, y, c.x, 260, c.x + 152, 260 + c.h, 34);
      if (ca > 0) blend(i, c.c, ca);
    }
  }
}
const raw = Buffer.alloc(S * (S * 4 + 1));
for (let y = 0; y < S; y++) px.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
const crcT = new Int32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});
const crc = (b) => {
  let c = -1;
  for (const v of b) c = crcT[(c ^ v) & 255] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const c = Buffer.alloc(4);
  c.writeUInt32BE(crc(td));
  return Buffer.concat([len, td, c]);
};
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8;
ihdr[9] = 6;
writeFileSync(
  OUT ?? "app-icon.png",
  Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]),
);
