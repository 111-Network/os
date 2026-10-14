// A small QR Code encoder for the dev mock and the tests (never shipped: the mock is reached only through the DEV-guarded
// dynamic import). It exists so the mock's own card is a real, scannable symbol: the Contacts view draws it, a phone can
// read it, and the scan path (jsQR) can be tested end to end without a camera.
//
// Byte mode, error correction L, versions 1 to 10, mask 0 (any mask is a valid symbol; the encoder skips the penalty
// search). The daemon produces the real matrix (`GET /contacts/me/card`); the UI only ever draws what it is given.
import type { Card } from "../types";

// [data codewords per block, number of blocks] groups, and EC codewords per block, for level L (ISO/IEC 18004 table 9).
const BLOCKS: Record<number, { ec: number; groups: [number, number][] }> = {
  1: { ec: 7, groups: [[19, 1]] },
  2: { ec: 10, groups: [[34, 1]] },
  3: { ec: 15, groups: [[55, 1]] },
  4: { ec: 20, groups: [[80, 1]] },
  5: { ec: 26, groups: [[108, 1]] },
  6: { ec: 18, groups: [[68, 2]] },
  7: { ec: 20, groups: [[78, 2]] },
  8: { ec: 24, groups: [[97, 2]] },
  9: { ec: 30, groups: [[116, 2]] },
  10: { ec: 18, groups: [[68, 2], [69, 2]] },
};

const ALIGNMENT: Record<number, number[]> = {
  1: [],
  2: [6, 18],
  3: [6, 22],
  4: [6, 26],
  5: [6, 30],
  6: [6, 34],
  7: [6, 22, 38],
  8: [6, 24, 42],
  9: [6, 26, 46],
  10: [6, 28, 50],
};

const dataCodewords = (v: number): number => BLOCKS[v]!.groups.reduce((n, [len, count]) => n + len * count, 0);

/** Largest byte payload per version at level L. */
export const qrCapacity = (v: number): number => dataCodewords(v) - (v < 10 ? 2 : 3);

// --- GF(256), polynomial 0x11D ---------------------------------------------------------------------------------

const EXP = new Uint8Array(512);
const LOG = new Uint8Array(256);
{
  let x = 1;
  for (let i = 0; i < 255; i++) {
    EXP[i] = x;
    LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255]!;
}
const mul = (a: number, b: number): number => (a === 0 || b === 0 ? 0 : EXP[LOG[a]! + LOG[b]!]!);

function rsEncode(data: readonly number[], ecLen: number): number[] {
  let gen = [1];
  for (let i = 0; i < ecLen; i++) {
    const next = new Array<number>(gen.length + 1).fill(0);
    gen.forEach((g, j) => {
      next[j] = (next[j] ?? 0) ^ g;
      next[j + 1] = (next[j + 1] ?? 0) ^ mul(g, EXP[i]!);
    });
    gen = next;
  }
  const rem = new Array<number>(ecLen).fill(0);
  for (const d of data) {
    const factor = d ^ rem[0]!;
    rem.shift();
    rem.push(0);
    for (let i = 0; i < ecLen; i++) rem[i] = rem[i]! ^ mul(gen[i + 1]!, factor);
  }
  return rem;
}

/** BCH remainder of `value` shifted by the generator's degree. */
function bch(value: number, poly: number, polyBits: number): number {
  let v = value << (polyBits - 1);
  for (let i = 31 - Math.clz32(v); i >= polyBits - 1; i--) if ((v >>> i) & 1) v ^= poly << (i - (polyBits - 1));
  return v;
}

// --- Symbol -----------------------------------------------------------------------------------------------------

/** The smallest version (1-10) that holds `n` bytes, or null. */
export function qrVersionFor(n: number): number | null {
  for (let v = 1; v <= 10; v++) if (n <= qrCapacity(v)) return v;
  return null;
}

/** Encode bytes as a QR symbol; rows of '0'/'1', top row first, no quiet zone. Throws when the data does not fit version 10. */
export function encodeQr(bytes: Uint8Array): { size: number; rows: string[] } {
  const v = qrVersionFor(bytes.length);
  if (v === null) throw new Error(`mock qr: ${bytes.length} bytes do not fit version 10`);

  // data bits
  const bits: number[] = [];
  const push = (value: number, n: number) => {
    for (let i = n - 1; i >= 0; i--) bits.push((value >>> i) & 1);
  };
  push(0b0100, 4);
  push(bytes.length, v < 10 ? 8 : 16);
  for (const b of bytes) push(b, 8);
  const capacityBits = dataCodewords(v) * 8;
  push(0, Math.min(4, capacityBits - bits.length));
  while (bits.length % 8) bits.push(0);
  const codewords: number[] = [];
  for (let i = 0; i < bits.length; i += 8) codewords.push(parseInt(bits.slice(i, i + 8).join(""), 2));
  for (let pad = 0xec; codewords.length < dataCodewords(v); pad ^= 0xec ^ 0x11) codewords.push(pad);

  // blocks, error correction, interleave
  const { ec, groups } = BLOCKS[v]!;
  const blocks: { data: number[]; ec: number[] }[] = [];
  let at = 0;
  for (const [len, count] of groups) {
    for (let i = 0; i < count; i++) {
      const data = codewords.slice(at, at + len);
      at += len;
      blocks.push({ data, ec: rsEncode(data, ec) });
    }
  }
  const stream: number[] = [];
  for (let i = 0; i < Math.max(...blocks.map((b) => b.data.length)); i++) for (const b of blocks) if (i < b.data.length) stream.push(b.data[i]!);
  for (let i = 0; i < ec; i++) for (const b of blocks) stream.push(b.ec[i]!);

  // matrix
  const size = 17 + 4 * v;
  const m: number[][] = Array.from({ length: size }, () => new Array<number>(size).fill(0));
  const fn: boolean[][] = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const set = (r: number, c: number, dark: boolean, isFn = true) => {
    m[r]![c] = dark ? 1 : 0;
    if (isFn) fn[r]![c] = true;
  };

  const finder = (r0: number, c0: number) => {
    for (let r = -1; r <= 7; r++)
      for (let c = -1; c <= 7; c++) {
        const rr = r0 + r;
        const cc = c0 + c;
        if (rr < 0 || cc < 0 || rr >= size || cc >= size) continue;
        const dark = r >= 0 && r <= 6 && c >= 0 && c <= 6 && (r === 0 || r === 6 || c === 0 || c === 6 || (r >= 2 && r <= 4 && c >= 2 && c <= 4));
        set(rr, cc, dark);
      }
  };
  finder(0, 0);
  finder(0, size - 7);
  finder(size - 7, 0);

  for (let i = 8; i < size - 8; i++) {
    set(6, i, i % 2 === 0);
    set(i, 6, i % 2 === 0);
  }
  const centers = ALIGNMENT[v]!;
  for (const r of centers)
    for (const c of centers) {
      if ((r === 6 && c === 6) || (r === 6 && c === size - 7) || (r === size - 7 && c === 6)) continue;
      for (let dr = -2; dr <= 2; dr++) for (let dc = -2; dc <= 2; dc++) set(r + dr, c + dc, Math.max(Math.abs(dr), Math.abs(dc)) !== 1);
    }
  set(size - 8, 8, true); // the dark module

  // reserve format and version areas, filled below
  for (let i = 0; i < 9; i++) {
    fn[8]![i] = true;
    fn[i]![8] = true;
  }
  for (let i = 0; i < 8; i++) {
    fn[8]![size - 1 - i] = true;
    fn[size - 1 - i]![8] = true;
  }
  if (v >= 7) {
    for (let i = 0; i < 6; i++)
      for (let j = 0; j < 3; j++) {
        fn[i]![size - 11 + j] = true;
        fn[size - 11 + j]![i] = true;
      }
  }

  // data, zigzag from the bottom right, mask 0
  let bit = 0;
  const total = stream.length * 8;
  let upward = true;
  for (let col = size - 1; col > 0; col -= 2) {
    if (col === 6) col--;
    for (let k = 0; k < size; k++) {
      const r = upward ? size - 1 - k : k;
      for (let dc = 0; dc < 2; dc++) {
        const c = col - dc;
        if (fn[r]![c]) continue;
        const dark = bit < total ? ((stream[bit >> 3]! >>> (7 - (bit & 7))) & 1) === 1 : false;
        bit++;
        m[r]![c] = (dark ? 1 : 0) ^ ((r + c) % 2 === 0 ? 1 : 0);
      }
    }
    upward = !upward;
  }

  // format information: level L (01), mask 0 (000)
  const format = (((0b01 << 3) | 0) << 10 | bch((0b01 << 3) | 0, 0x537, 11)) ^ 0x5412;
  const fbit = (i: number) => ((format >>> i) & 1) === 1;
  for (let i = 0; i <= 5; i++) set(i, 8, fbit(i));
  set(7, 8, fbit(6));
  set(8, 8, fbit(7));
  set(8, 7, fbit(8));
  for (let i = 9; i < 15; i++) set(8, 14 - i, fbit(i));
  for (let i = 0; i < 8; i++) set(8, size - 1 - i, fbit(i));
  for (let i = 8; i < 15; i++) set(size - 15 + i, 8, fbit(i));

  // version information (versions 7 and up)
  if (v >= 7) {
    const info = (v << 12) | bch(v, 0x1f25, 13);
    for (let i = 0; i < 18; i++) {
      const dark = ((info >>> i) & 1) === 1;
      const a = Math.floor(i / 3);
      const b = (i % 3) + size - 11;
      set(a, b, dark);
      set(b, a, dark);
    }
  }

  return { size, rows: m.map((row) => row.join("")) };
}

/** A `Card` for `text`: the text itself and the matrix of its UTF-8 bytes. */
export function cardFor(text: string): Card {
  return { text, qr: encodeQr(new TextEncoder().encode(text)) };
}
