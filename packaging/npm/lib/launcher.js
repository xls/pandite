// The Pandite npm launcher. On first run it downloads the Pandite release
// file for this system and for the exact version of this package, checks
// its SHA-256 against checksums.json, unpacks it into a per-user cache and
// then runs the program with the given arguments. Later runs use the cache.
// Node.js standard library only. Every function takes its system and
// network dependencies as arguments, so test.mjs runs it offline.

import { spawn as nodeSpawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { chmod, mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import https from 'node:https';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';
import { gunzipSync, inflateRawSync } from 'node:zlib';

export const RELEASES_URL = 'https://github.com/xls/pandite/releases/download';
export const BUILD_DOCS = 'https://github.com/xls/pandite/blob/main/docs/building.md';
export const MIN_GLIBC = '2.35';
export const SUPPORTED =
  'Windows x64, Windows ARM64, macOS 11 or later (Apple Silicon and Intel) and Linux x64 with glibc 2.35 or later';
export const MAX_REDIRECTS = 5;
export const MAX_DOWNLOAD_BYTES = 512 * 1024 * 1024;
const MARKER = '.pandite-npm.json';
const REDIRECT_CODES = new Set([301, 302, 303, 307, 308]);
const PACKAGE_DIR = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const OS_NAMES = { win32: 'Windows', darwin: 'macOS', linux: 'Linux' };

/** Every system with a Pandite build, as Node.js names it. */
export const PLATFORMS = [
  { platform: 'win32', arch: 'x64', glibc: null },
  { platform: 'win32', arch: 'arm64', glibc: null },
  { platform: 'darwin', arch: 'x64', glibc: null },
  { platform: 'darwin', arch: 'arm64', glibc: null },
  { platform: 'linux', arch: 'x64', glibc: MIN_GLIBC },
];

/** An error with a message for the user; the launcher prints no stack. */
export class LauncherError extends Error {
  constructor(message) {
    super(message);
    this.name = 'LauncherError';
  }
}

function versionBelow(have, want) {
  const a = String(have).split('.').map((n) => Number.parseInt(n, 10) || 0);
  const b = want.split('.').map(Number);
  for (let i = 0; i < b.length; i += 1) {
    const x = a[i] ?? 0;
    if (x !== b[i]) return x < b[i];
  }
  return false;
}

/**
 * The release file for a system: `asset` is the file name on the release
 * page, `exe` the program inside it, `target` the cache folder name.
 * Throws a LauncherError that names the supported systems otherwise.
 */
export function assetFor({ platform, arch, version, glibc }) {
  const system = `${OS_NAMES[platform] ?? platform} on ${arch}`;
  const unsupported = (why) =>
    new LauncherError(`${why}. Pandite builds exist for ${SUPPORTED}. To build Pandite yourself, see ${BUILD_DOCS}`);
  if (platform === 'win32') {
    const target = { x64: 'x86_64-pc-windows-msvc', arm64: 'aarch64-pc-windows-msvc' }[arch];
    if (!target) throw unsupported(`There is no Pandite build for ${system}`);
    const dir = `pandite-${version}-${target}`;
    return { target, asset: `${dir}.zip`, format: 'zip', exe: `${dir}/pandite.exe` };
  }
  if (platform === 'darwin') {
    if (arch !== 'x64' && arch !== 'arm64') throw unsupported(`There is no Pandite build for ${system}`);
    const target = 'universal-apple-darwin';
    return { target, asset: `pandite-${version}-${target}.zip`, format: 'zip', exe: 'pandite.app/Contents/MacOS/pandite' };
  }
  if (platform === 'linux' && arch === 'x64') {
    if (!glibc) throw unsupported('The Linux build needs glibc, and this system uses another C library (for example musl)');
    if (versionBelow(glibc, MIN_GLIBC)) {
      throw unsupported(`The Linux build needs glibc ${MIN_GLIBC} or later, and this system has glibc ${glibc}`);
    }
    const target = 'x86_64-unknown-linux-gnu';
    const dir = `pandite-${version}-${target}`;
    return { target, asset: `${dir}.tar.gz`, format: 'tar.gz', exe: `${dir}/pandite` };
  }
  throw unsupported(`There is no Pandite build for ${system}`);
}

/** The release files of every supported system, without repeats. */
export function allAssets(version) {
  return [...new Set(PLATFORMS.map((p) => assetFor({ ...p, version }).asset))];
}

/** The glibc version of this Linux system, or null (musl, or not Linux). */
export function detectGlibc() {
  try {
    return process.report?.getReport?.()?.header?.glibcVersionRuntime ?? null;
  } catch {
    return null;
  }
}

/** The per-user folder for the unpacked programs. */
export function cacheRoot({ env, platform, home }) {
  if (env.PANDITE_NPM_CACHE) return path.resolve(env.PANDITE_NPM_CACHE);
  if (platform === 'win32') return path.join(env.LOCALAPPDATA || path.join(home, 'AppData', 'Local'), 'pandite', 'npm');
  if (platform === 'darwin') return path.join(home, 'Library', 'Caches', 'pandite', 'npm');
  const xdg = env.XDG_CACHE_HOME && path.isAbsolute(env.XDG_CACHE_HOME) ? env.XDG_CACHE_HOME : path.join(home, '.cache');
  return path.join(xdg, 'pandite', 'npm');
}

/** One HTTPS GET without redirects: { statusCode, headers, body }. */
export function httpsRequest(url, { headers = {}, timeoutMs = 60_000 } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers }, (res) => {
      resolve({ statusCode: res.statusCode ?? 0, headers: res.headers, body: res });
    });
    req.setTimeout(timeoutMs, () => {
      req.destroy(new LauncherError(`no data from ${new URL(url).host} for ${timeoutMs / 1000} seconds`));
    });
    req.on('error', reject);
  });
}

/**
 * Download `url` to the new file `file`, following at most MAX_REDIRECTS
 * redirects, each to an HTTPS address only. Returns { sha256, bytes }.
 * `request` does one GET; tests pass a fake.
 */
export async function download(url, file, { request = httpsRequest, userAgent = 'pandite-npm-launcher', maxBytes = MAX_DOWNLOAD_BYTES } = {}) {
  let current = new URL(url);
  for (let hops = 0; ; hops += 1) {
    if (current.protocol !== 'https:') {
      const what = hops === 0 ? 'to download' : 'the redirect to';
      throw new LauncherError(`refusing ${what} ${current.href}: only HTTPS addresses are allowed`);
    }
    let res;
    try {
      res = await request(current.href, { headers: { 'user-agent': userAgent, accept: 'application/octet-stream' } });
    } catch (error) {
      throw new LauncherError(`cannot download ${current.href}: ${error.message}`);
    }
    if (REDIRECT_CODES.has(res.statusCode)) {
      res.body?.resume?.();
      const location = res.headers?.location;
      if (!location) throw new LauncherError(`${current.href} answered ${res.statusCode} without a Location header`);
      if (hops >= MAX_REDIRECTS) throw new LauncherError(`more than ${MAX_REDIRECTS} redirects for ${url}`);
      current = new URL(location, current);
      continue;
    }
    if (res.statusCode !== 200) {
      res.body?.resume?.();
      const hint = res.statusCode === 404 ? ' (the release or the file does not exist)' : '';
      throw new LauncherError(`download of ${url} failed: HTTP ${res.statusCode}${hint}`);
    }
    const hash = createHash('sha256');
    let bytes = 0;
    const meter = new Transform({
      transform(chunk, _encoding, done) {
        bytes += chunk.length;
        if (bytes > maxBytes) {
          done(new LauncherError(`${url} is larger than ${maxBytes} bytes`));
          return;
        }
        hash.update(chunk);
        done(null, chunk);
      },
    });
    try {
      await pipeline(res.body, meter, createWriteStream(file, { flags: 'wx' }));
    } catch (error) {
      if (error instanceof LauncherError) throw error;
      throw new LauncherError(`download of ${url} failed: ${error.message}`);
    }
    return { sha256: hash.digest('hex'), bytes };
  }
}

// ------------------------------------------------------------------ unpack

/** The parts of an archive entry name; refuses names that leave the folder. */
function entryParts(name) {
  const clean = name.replace(/\\/g, '/');
  if (clean.startsWith('/') || /^[A-Za-z]:/.test(clean)) throw new LauncherError(`archive entry with an absolute path: ${name}`);
  const parts = clean.split('/').filter((p) => p !== '' && p !== '.');
  if (parts.some((p) => p === '..' || /[:\0]/.test(p))) throw new LauncherError(`archive entry outside its folder: ${name}`);
  return parts;
}

function writeEntry(dest, parts, data, mode) {
  const file = path.join(dest, ...parts);
  mkdirSync(path.dirname(file), { recursive: true });
  // "wx": an entry never replaces another entry or follows a link.
  writeFileSync(file, data, { mode: mode || 0o644, flag: 'wx' });
}

/** Unpack a zip file (stored or deflated entries) into `dest`. */
export function unzip(buf, dest) {
  let eocd = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 22 - 0xffff); i -= 1) {
    if (buf.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new LauncherError('not a zip file');
  const count = buf.readUInt16LE(eocd + 10);
  const cdSize = buf.readUInt32LE(eocd + 12);
  const cdOffset = buf.readUInt32LE(eocd + 16);
  if (count === 0xffff || cdOffset === 0xffffffff) throw new LauncherError('zip64 files are not supported');
  if (cdOffset + cdSize > eocd) throw new LauncherError('damaged zip file');
  const files = [];
  let p = cdOffset;
  for (let n = 0; n < count; n += 1) {
    if (p + 46 > eocd || buf.readUInt32LE(p) !== 0x02014b50) throw new LauncherError('damaged zip directory');
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
    if (flags & 0x1) throw new LauncherError(`encrypted zip entry: ${name}`);
    const parts = entryParts(name);
    // macOS keeps extended attributes in __MACOSX; the program needs none.
    if (parts.length === 0 || parts[0] === '__MACOSX') continue;
    const unixMode = madeBy >> 8 === 3 ? external >>> 16 : 0;
    const type = unixMode & 0o170000;
    if (name.endsWith('/') || type === 0o040000) {
      mkdirSync(path.join(dest, ...parts), { recursive: true });
      continue;
    }
    if (type !== 0 && type !== 0o100000) throw new LauncherError(`zip entry is not a file or folder: ${name}`);
    if (local + 30 > buf.length || buf.readUInt32LE(local) !== 0x04034b50) throw new LauncherError(`damaged zip entry: ${name}`);
    const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
    const raw = buf.subarray(start, start + compressed);
    if (raw.length !== compressed) throw new LauncherError(`damaged zip entry: ${name}`);
    let data;
    if (method === 0) data = raw;
    else if (method === 8) data = inflateRawSync(raw);
    else throw new LauncherError(`zip entry ${name} uses compression method ${method}`);
    if (data.length !== size) throw new LauncherError(`zip entry ${name} has the wrong size`);
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

/** Unpack a .tar.gz file (ustar, GNU long names, pax paths) into `dest`. */
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
      if (header[start] & 0x80) throw new LauncherError('tar entry too large');
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
    if (data.length !== size) throw new LauncherError('damaged tar file');
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
    if (type !== '0' && type !== '7') throw new LauncherError(`tar entry is not a file or folder: ${name}`);
    if (parts.length === 0) continue;
    writeEntry(dest, parts, data, mode & 0o777);
    files.push(parts.join('/'));
  }
  return files;
}

// ------------------------------------------------------------------- cache

async function isComplete(dir, pick, sha256) {
  let marker;
  try {
    marker = JSON.parse(await readFile(path.join(dir, MARKER), 'utf8'));
  } catch {
    return false;
  }
  return marker?.asset === pick.asset && marker?.sha256 === sha256 && existsSync(path.join(dir, ...pick.exe.split('/')));
}

/**
 * The program for this system, from the cache, or downloaded, checked and
 * unpacked into the cache first. Returns { exe, downloaded }.
 */
export async function ensureProgram({ version, checksums, platform, arch, glibc, env, home, request, log }) {
  const pick = assetFor({ platform, arch, version, glibc });
  const expected = checksums?.sha256?.[pick.asset];
  if (typeof expected !== 'string' || !/^[0-9a-f]{64}$/.test(expected)) {
    throw new LauncherError(`this launcher has no SHA-256 for ${pick.asset}; run the published package: npx pandite@${version}`);
  }
  const versionDir = path.join(cacheRoot({ env, platform, home }), version);
  const dir = path.join(versionDir, pick.target);
  const exe = path.join(dir, ...pick.exe.split('/'));
  if (await isComplete(dir, pick, expected)) return { exe, downloaded: false };

  await mkdir(versionDir, { recursive: true });
  const work = await mkdtemp(path.join(versionDir, '.download-'));
  try {
    const archive = path.join(work, pick.asset);
    const url = `${RELEASES_URL}/v${version}/${pick.asset}`;
    log(`first run of Pandite ${version}: downloading ${url}`);
    const got = await download(url, archive, { request, userAgent: `pandite-npm-launcher/${version}` });
    if (got.sha256 !== expected) {
      throw new LauncherError(
        `SHA-256 mismatch for ${pick.asset}: expected ${expected}, received ${got.sha256}. The file was deleted and not run.`,
      );
    }
    const unpacked = path.join(work, 'unpacked');
    mkdirSync(unpacked);
    const data = await readFile(archive);
    if (pick.format === 'zip') unzip(data, unpacked);
    else untar(data, unpacked);
    const unpackedExe = path.join(unpacked, ...pick.exe.split('/'));
    if (!existsSync(unpackedExe)) throw new LauncherError(`${pick.asset} does not contain ${pick.exe}`);
    if (platform !== 'win32') await chmod(unpackedExe, 0o755);
    await writeFile(path.join(unpacked, MARKER), `${JSON.stringify({ asset: pick.asset, sha256: expected })}\n`);
    // Another launcher can finish the same download first; keep its copy.
    if (!(await isComplete(dir, pick, expected))) {
      await rm(dir, { recursive: true, force: true });
      try {
        await rename(unpacked, dir);
      } catch (error) {
        if (!(await isComplete(dir, pick, expected))) throw error;
      }
    }
    log(`Pandite ${version} is in ${dir}`);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
  return { exe, downloaded: true };
}

// ------------------------------------------------------------------ launch

/**
 * True when the launcher starts the program and returns at once. On
 * Windows the program stays in the background after its window closes, so
 * a run that only opens images returns like a start from Explorer. A run
 * with an option (--version, --register, ...) waits for the exit code.
 */
export function startsDetached(platform, args) {
  if (platform !== 'win32') return false;
  let onlyPaths = false;
  for (const arg of args) {
    if (onlyPaths || arg === '--background') continue;
    if (arg === '--') {
      onlyPaths = true;
      continue;
    }
    if ((arg.startsWith('-') && arg.length > 1) || arg === '/?') return false;
  }
  return true;
}

/** Run the program. Resolves { code, signal } (code 0 for a detached start). */
export function launch(exe, args, { platform, spawn = nodeSpawn }) {
  return new Promise((resolve, reject) => {
    const failed = (error) => reject(new LauncherError(`cannot start ${exe}: ${error.message}`));
    if (startsDetached(platform, args)) {
      const child = spawn(exe, args, { detached: true, stdio: 'ignore' });
      child.once('error', failed);
      child.once('spawn', () => {
        child.unref();
        resolve({ code: 0, signal: null });
      });
      return;
    }
    const child = spawn(exe, args, { stdio: 'inherit' });
    // The terminal sends Ctrl+C to the program as well; on Windows the
    // program has no console, so the launcher ends it.
    const handlers = ['SIGINT', 'SIGTERM', 'SIGHUP'].map((signal) => [
      signal,
      () => {
        if (signal !== 'SIGINT' || platform === 'win32') child.kill(signal === 'SIGINT' ? 'SIGTERM' : signal);
      },
    ]);
    for (const [signal, handler] of handlers) process.on(signal, handler);
    const done = () => {
      for (const [signal, handler] of handlers) process.off(signal, handler);
    };
    child.once('error', (error) => {
      done();
      failed(error);
    });
    child.once('exit', (code, signal) => {
      done();
      resolve({ code, signal });
    });
  });
}

// -------------------------------------------------------------------- main

function readJson(file, missing) {
  let text;
  try {
    text = readFileSync(file, 'utf8');
  } catch {
    throw new LauncherError(missing);
  }
  return JSON.parse(text);
}

/**
 * The launcher: `args` go to the program. `deps` replaces the system and
 * the network for tests: platform, arch, glibc, env, home, version,
 * checksums, request, spawn, log.
 */
export async function main(args, deps = {}) {
  const platform = deps.platform ?? process.platform;
  const arch = deps.arch ?? process.arch;
  const glibc = 'glibc' in deps ? deps.glibc : platform === 'linux' ? detectGlibc() : null;
  const version =
    deps.version ?? readJson(path.join(PACKAGE_DIR, 'package.json'), 'the launcher package has no package.json').version;
  // Refuse an unsupported system before anything else.
  assetFor({ platform, arch, version, glibc });
  const checksums =
    deps.checksums ??
    readJson(
      path.join(PACKAGE_DIR, 'checksums.json'),
      'this copy of the launcher has no checksums.json (the release workflow adds it); run the published package: npx pandite',
    );
  if (checksums?.version !== version) {
    throw new LauncherError(`checksums.json is for version ${checksums?.version}, the launcher is version ${version}`);
  }
  const { exe } = await ensureProgram({
    version,
    checksums,
    platform,
    arch,
    glibc,
    env: deps.env ?? process.env,
    home: deps.home ?? os.homedir(),
    request: deps.request ?? httpsRequest,
    log: deps.log ?? ((message) => process.stderr.write(`pandite: ${message}\n`)),
  });
  return launch(exe, args, { platform, spawn: deps.spawn ?? nodeSpawn });
}
