// Unpack zip and .tar.gz release files with the Node.js standard library.
// Every entry must stay inside the target folder; links are refused.

import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { gunzipSync, inflateRawSync } from 'node:zlib';
import { InstallerError } from './errors.js';

/** The parts of an archive entry name; refuses names that leave the folder. */
function entryParts(name) {
  const clean = name.replace(/\\/g, '/');
  if (clean.startsWith('/') || /^[A-Za-z]:/.test(clean)) throw new InstallerError(`archive entry with an absolute path: ${name}`);
  const parts = clean.split('/').filter((p) => p !== '' && p !== '.');
  if (parts.some((p) => p === '..' || /[:\0]/.test(p))) throw new InstallerError(`archive entry outside its folder: ${name}`);
  return parts;
}

function writeEntry(dest, parts, data, mode) {
  const file = path.join(dest, ...parts);
  mkdirSync(path.dirname(file), { recursive: true });
  // "wx": an entry never replaces another entry or follows a link.
  writeFileSync(file, data, { mode: mode || 0o644, flag: 'wx' });
}

/** Unpack a zip file (stored or deflated entries) into `dest`. Returns the file names. */
export function unzip(buf, dest) {
  let eocd = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 22 - 0xffff); i -= 1) {
    if (buf.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new InstallerError('not a zip file');
  const count = buf.readUInt16LE(eocd + 10);
  const cdSize = buf.readUInt32LE(eocd + 12);
  const cdOffset = buf.readUInt32LE(eocd + 16);
  if (count === 0xffff || cdOffset === 0xffffffff) throw new InstallerError('zip64 files are not supported');
  if (cdOffset + cdSize > eocd) throw new InstallerError('damaged zip file');
  const files = [];
  let p = cdOffset;
  for (let n = 0; n < count; n += 1) {
    if (p + 46 > eocd || buf.readUInt32LE(p) !== 0x02014b50) throw new InstallerError('damaged zip directory');
    const madeBy = buf.readUInt16LE(p + 4);
    const flags = buf.readUInt16LE(p + 8);
    const method = buf.readUInt16LE(p + 10);
    const compressed = buf.readUInt32LE(p + 20);
    const size = buf.readUInt32LE(p + 24);
    const nameLength = buf.readUInt16LE(p + 28);
    const extraLength = buf.readUInt16LE(p + 30);
    const commentLength = buf.readUInt16LE(p + 32);
    const external = buf.readUInt32LE(p + 38);
    const local = buf.readUInt32LE(p + 42);
    const name = buf.toString(flags & 0x800 ? 'utf8' : 'latin1', p + 46, p + 46 + nameLength);
    p += 46 + nameLength + extraLength + commentLength;
    if (flags & 0x1) throw new InstallerError(`encrypted zip entry: ${name}`);
    const parts = entryParts(name);
    // macOS keeps extended attributes in __MACOSX; the app needs none.
    if (parts.length === 0 || parts[0] === '__MACOSX') continue;
    const unixMode = madeBy >> 8 === 3 ? external >>> 16 : 0;
    const type = unixMode & 0o170000;
    if (name.endsWith('/') || type === 0o040000) {
      mkdirSync(path.join(dest, ...parts), { recursive: true });
      continue;
    }
    if (type !== 0 && type !== 0o100000) throw new InstallerError(`zip entry is not a file or folder: ${name}`);
    if (local + 30 > buf.length || buf.readUInt32LE(local) !== 0x04034b50) throw new InstallerError(`damaged zip entry: ${name}`);
    const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
    const raw = buf.subarray(start, start + compressed);
    if (raw.length !== compressed) throw new InstallerError(`damaged zip entry: ${name}`);
    let data;
    if (method === 0) data = raw;
    else if (method === 8) data = inflateRawSync(raw);
    else throw new InstallerError(`zip entry ${name} uses compression method ${method}`);
    if (data.length !== size) throw new InstallerError(`zip entry ${name} has the wrong size`);
    writeEntry(dest, parts, data, unixMode & 0o777);
    files.push(parts.join('/'));
  }
  return files;
}

function paxPath(data) {
  let value = null;
  let i = 0;
  while (i < data.length) {
    const space = data.indexOf(0x20, i);
    if (space < 0) break;
    const length = Number.parseInt(data.toString('latin1', i, space), 10);
    if (!(length > 0) || i + length > data.length) break;
    const record = data.toString('utf8', space + 1, i + length - 1);
    const eq = record.indexOf('=');
    if (eq > 0 && record.slice(0, eq) === 'path') value = record.slice(eq + 1);
    i += length;
  }
  return value;
}

/** Unpack a .tar.gz file (ustar, GNU long names, pax paths) into `dest`. Returns the file names. */
export function untar(gz, dest) {
  const buf = gunzipSync(gz);
  const files = [];
  let longName = null;
  let p = 0;
  while (p + 512 <= buf.length) {
    const header = buf.subarray(p, p + 512);
    if (header.every((b) => b === 0)) break;
    const text = (start, length) => {
      const field = header.subarray(start, start + length);
      const end = field.indexOf(0);
      return field.toString('utf8', 0, end < 0 ? length : end);
    };
    const octal = (start, length) => {
      if (header[start] & 0x80) throw new InstallerError('tar entry too large');
      const digits = text(start, length).trim();
      return digits ? Number.parseInt(digits, 8) : 0;
    };
    const size = octal(124, 12);
    const mode = octal(100, 8);
    const type = header[156] === 0 ? '0' : String.fromCharCode(header[156]);
    let name = text(0, 100);
    // POSIX ustar splits long names into prefix and name; GNU tar uses "L".
    if (header.toString('latin1', 257, 263) === 'ustar\u0000') {
      const prefix = text(345, 155);
      if (prefix) name = `${prefix}/${name}`;
    }
    const data = buf.subarray(p + 512, p + 512 + size);
    if (data.length !== size) throw new InstallerError('damaged tar file');
    p += 512 + Math.ceil(size / 512) * 512;
    if (type === 'L') {
      longName = data.toString('utf8').replace(/\0+$/, '');
      continue;
    }
    if (type === 'x') {
      longName = paxPath(data) ?? longName;
      continue;
    }
    if (type === 'g') continue;
    if (longName !== null) {
      name = longName;
      longName = null;
    }
    const parts = entryParts(name);
    if (type === '5') {
      if (parts.length) mkdirSync(path.join(dest, ...parts), { recursive: true });
      continue;
    }
    if (type !== '0' && type !== '7') throw new InstallerError(`tar entry is not a file or folder: ${name}`);
    if (parts.length === 0) continue;
    writeEntry(dest, parts, data, mode & 0o777);
    files.push(parts.join('/'));
  }
  return files;
}
