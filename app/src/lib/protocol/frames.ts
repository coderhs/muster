// Binary terminal frames: 0x01 | u16 BE sid length | sid (utf-8) | raw bytes.

export const TERM_FRAME = 0x01;

const enc = new TextEncoder();
const dec = new TextDecoder();

export interface TermFrame {
  sid: string;
  data: Uint8Array;
}

export function decodeTermFrame(buf: ArrayBuffer | Uint8Array): TermFrame | null {
  const b = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  if (b.length < 3 || b[0] !== TERM_FRAME) return null;
  const len = (b[1] << 8) | b[2];
  if (b.length < 3 + len) return null;
  return { sid: dec.decode(b.subarray(3, 3 + len)), data: b.subarray(3 + len) };
}

export function encodeTermFrame(sid: string, data: Uint8Array | string): Uint8Array {
  const s = enc.encode(sid);
  const d = typeof data === "string" ? enc.encode(data) : data;
  const out = new Uint8Array(3 + s.length + d.length);
  out[0] = TERM_FRAME;
  out[1] = (s.length >> 8) & 0xff;
  out[2] = s.length & 0xff;
  out.set(s, 3);
  out.set(d, 3 + s.length);
  return out;
}
