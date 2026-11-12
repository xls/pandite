// Minimal PNG reader and writer for the packaging scripts.
//
// Standard library only (node:zlib). The reader handles non-interlaced
// images with 8 or 16 bits per channel in every color type, plus palette
// and gray images with 1, 2 or 4 bits. The writer produces 8-bit RGBA with
// adaptive filtering. Output is deterministic: no time or text chunks.

import { deflateSync, inflateSync } from 'node:zlib';

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

/** CRC-32 (ISO 3309), as PNG and ZIP use it. */
export function crc32(bytes, start = 0, end = bytes.length) {
  let c = 0xffffffff;
  for (let i = start; i < end; i += 1) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'latin1');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out, 4, 8 + data.length), 8 + data.length);
  return out;
}

function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  if (pa <= pb && pa <= pc) return a;
  return pb <= pc ? b : c;
}

/**
 * Encode 8-bit straight-alpha RGBA pixels as a PNG file.
 * @param {number} width
 * @param {number} height
 * @param {Uint8Array} rgba width * height * 4 bytes
 * @returns {Buffer}
 */
export function encodePng(width, height, rgba) {
  if (rgba.length !== width * height * 4) throw new Error('encodePng: pixel buffer size mismatch');
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  const candidate = Buffer.alloc(stride);
  const zero = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const row = rgba.subarray(y * stride, (y + 1) * stride);
    const up = y > 0 ? rgba.subarray((y - 1) * stride, y * stride) : zero;
    let bestType = 0;
    let bestScore = Infinity;
    let best = null;
    for (let type = 0; type < 5; type += 1) {
      let score = 0;
      for (let i = 0; i < stride; i += 1) {
        const a = i >= 4 ? row[i - 4] : 0;
        const b = up[i];
        const c = i >= 4 ? up[i - 4] : 0;
        let v;
        switch (type) {
          case 0: v = row[i]; break;
          case 1: v = row[i] - a; break;
          case 2: v = row[i] - b; break;
          case 3: v = row[i] - ((a + b) >> 1); break;
          default: v = row[i] - paeth(a, b, c); break;
        }
        v &= 0xff;
        candidate[i] = v;
        score += v < 128 ? v : 256 - v;
      }
      if (score < bestScore) {
        bestScore = score;
        bestType = type;
        best = Buffer.from(candidate);
      }
    }
    raw[y * (stride + 1)] = bestType;
    best.copy(raw, y * (stride + 1) + 1);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  return Buffer.concat([
    SIGNATURE,
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9, memLevel: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/**
 * Decode a PNG file to 8-bit straight-alpha RGBA.
 * @param {Uint8Array} file
 * @returns {{ width: number, height: number, rgba: Uint8Array }}
 */
export function decodePng(file) {
  const buf = Buffer.from(file.buffer, file.byteOffset, file.byteLength);
  if (buf.length < 8 || !buf.subarray(0, 8).equals(SIGNATURE)) throw new Error('not a PNG file');
  let pos = 8;
  let width = 0;
  let height = 0;
  let depth = 0;
  let colorType = 0;
  let interlace = 0;
  let palette = null;
  let trns = null;
  const idat = [];
  while (pos + 8 <= buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('latin1', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (data.length !== len) throw new Error('truncated PNG chunk');
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      depth = data[8];
      colorType = data[9];
      interlace = data[12];
    } else if (type === 'PLTE') {
      palette = data;
    } else if (type === 'tRNS') {
      trns = data;
    } else if (type === 'IDAT') {
      idat.push(data);
    } else if (type === 'IEND') {
      break;
    }
    pos += 12 + len;
  }
  if (!width || !height) throw new Error('PNG has no IHDR');
  if (interlace !== 0) throw new Error('interlaced PNG is not supported');
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[colorType];
  if (!channels) throw new Error(`unsupported PNG color type ${colorType}`);
  if (colorType === 3 && !palette) throw new Error('palette PNG without PLTE');
  const bitsPerPixel = channels * depth;
  const bpp = Math.max(1, bitsPerPixel >> 3);
  const stride = Math.ceil((width * bitsPerPixel) / 8);
  const raw = inflateSync(Buffer.concat(idat));
  if (raw.length < (stride + 1) * height) throw new Error('PNG image data is too short');

  const cur = Buffer.alloc(stride);
  const prev = Buffer.alloc(stride);
  const rgba = new Uint8Array(width * height * 4);
  const maxSample = (1 << Math.min(depth, 8)) - 1;
  for (let y = 0; y < height; y += 1) {
    const base = y * (stride + 1);
    const filter = raw[base];
    for (let i = 0; i < stride; i += 1) {
      const x = raw[base + 1 + i];
      const a = i >= bpp ? cur[i - bpp] : 0;
      const b = prev[i];
      const c = i >= bpp ? prev[i - bpp] : 0;
      let v;
      switch (filter) {
        case 0: v = x; break;
        case 1: v = x + a; break;
        case 2: v = x + b; break;
        case 3: v = x + ((a + b) >> 1); break;
        case 4: v = x + paeth(a, b, c); break;
        default: throw new Error(`bad PNG filter type ${filter}`);
      }
      cur[i] = v & 0xff;
    }
    const sample = (index) => {
      // Sample number `index` of this row, scaled to 8 bits.
      if (depth === 8) return cur[index];
      if (depth === 16) return cur[index * 2];
      const bit = index * depth;
      const v = (cur[bit >> 3] >> (8 - depth - (bit & 7))) & maxSample;
      return colorType === 3 ? v : Math.round((v * 255) / maxSample);
    };
    const raw16 = (index) => (cur[index * 2] << 8) | cur[index * 2 + 1];
    for (let x = 0; x < width; x += 1) {
      const o = (y * width + x) * 4;
      let r;
      let g;
      let bl;
      let al = 255;
      if (colorType === 3) {
        const idx = sample(x);
        r = palette[idx * 3];
        g = palette[idx * 3 + 1];
        bl = palette[idx * 3 + 2];
        if (trns && idx < trns.length) al = trns[idx];
      } else if (colorType === 0 || colorType === 4) {
        r = g = bl = sample(x * channels);
        if (colorType === 4) al = sample(x * channels + 1);
        else if (trns) {
          const key = trns.readUInt16BE(0);
          const value = depth === 16 ? raw16(x) : (depth === 8 ? cur[x] : sample(x) * maxSample / 255);
          if (Math.round(value) === key) al = 0;
        }
      } else {
        r = sample(x * channels);
        g = sample(x * channels + 1);
        bl = sample(x * channels + 2);
        if (colorType === 6) al = sample(x * channels + 3);
        else if (trns) {
          const k = [trns.readUInt16BE(0), trns.readUInt16BE(2), trns.readUInt16BE(4)];
          const v = depth === 16
            ? [raw16(x * 3), raw16(x * 3 + 1), raw16(x * 3 + 2)]
            : [cur[x * 3], cur[x * 3 + 1], cur[x * 3 + 2]];
          if (v[0] === k[0] && v[1] === k[1] && v[2] === k[2]) al = 0;
        }
      }
      rgba[o] = r;
      rgba[o + 1] = g;
      rgba[o + 2] = bl;
      rgba[o + 3] = al;
    }
    cur.copy(prev);
  }
  return { width, height, rgba };
}
