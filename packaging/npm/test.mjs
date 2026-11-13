#!/usr/bin/env node
// Offline test of the npm launcher. No network: a fake request function
// serves the release files from memory, a fake spawn records the program
// start, and every file goes to a temporary folder.
//
// Usage: node packaging/npm/test.mjs
//
// Proves: the release file of every supported system, a clear refusal of
// unsupported systems, download through an HTTPS redirect, unpacking of
// zip and tar.gz, the refusal of a file with another SHA-256, of a redirect
// to plain HTTP and of archive entries outside their folder, the reuse of
// the cache on the second run, the exit code, and checksums.json from a
// SHA256SUMS.txt. Prints "NPM LAUNCHER VERIFIED" on success.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { Readable } from 'node:stream';
import { deflateRawSync, gzipSync } from 'node:zlib';
import { makeChecksums, parseSums } from './build.mjs';
import {
  LauncherError,
  PLATFORMS,
  allAssets,
  assetFor,
  download,
  main,
  startsDetached,
  untar,
  unzip,
} from './lib/launcher.js';

const VERSION = '1.2.3';
const TEMP = mkdtempSync(path.join(os.tmpdir(), 'pandite-npm-test-'));
let tempCount = 0;
const tempDir = () => {
  tempCount += 1;
  return path.join(TEMP, String(tempCount));
};
const sha256 = (buf) => createHash('sha256').update(buf).digest('hex');

// ------------------------------------------------------- archive writers

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** entries: { name, data?, mode? }; a name ending in "/" is a folder. */
function makeZip(entries) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const e of entries) {
    const name = Buffer.from(e.name, 'utf8');
    const data = e.data ?? Buffer.alloc(0);
    const packed = e.name.endsWith('/') ? data : deflateRawSync(data);
    const method = e.name.endsWith('/') ? 0 : 8;
    const mode = e.mode ?? (e.name.endsWith('/') ? 0o040755 : 0o100644);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x800, 6);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(crc32(data), 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE((3 << 8) | 20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x800, 8);
    central.writeUInt16LE(method, 10);
    central.writeUInt32LE(crc32(data), 16);
    central.writeUInt32LE(packed.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt32LE((mode << 16) >>> 0, 38);
    central.writeUInt32LE(offset, 42);
    locals.push(local, name, packed);
    centrals.push(central, name);
    offset += local.length + name.length + packed.length;
  }
  const cd = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, cd, end]);
}

function tarHeader(name, size, mode, type) {
  const h = Buffer.alloc(512);
  h.write(name, 0, 100, 'utf8');
  h.write(`${mode.toString(8).padStart(7, '0')}\0`, 100, 'latin1');
  h.write('0000000\0', 108, 'latin1');
  h.write('0000000\0', 116, 'latin1');
  h.write(`${size.toString(8).padStart(11, '0')}\0`, 124, 'latin1');
  h.write('00000000000\0', 136, 'latin1');
  h.write(type, 156, 'latin1');
  h.write('ustar  \0', 257, 'latin1');
  h.fill(0x20, 148, 156);
  let sum = 0;
  for (const b of h) sum += b;
  h.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148, 'latin1');
  return h;
}

/** GNU-style .tar.gz; names over 100 bytes get an "L" long-name entry. */
function makeTarGz(entries) {
  const blocks = [];
  const pad = (n) => Buffer.alloc((512 - (n % 512)) % 512);
  for (const e of entries) {
    const dir = e.name.endsWith('/');
    const data = e.data ?? Buffer.alloc(0);
    if (Buffer.byteLength(e.name) > 100) {
      const long = Buffer.from(`${e.name}\0`, 'utf8');
      blocks.push(tarHeader('././@LongLink', long.length, 0o644, 'L'), long, pad(long.length));
    }
    blocks.push(tarHeader(e.name.slice(0, 100), dir ? 0 : data.length, e.mode ?? (dir ? 0o755 : 0o644), e.type ?? (dir ? '5' : '0')));
    if (!dir) blocks.push(data, pad(data.length));
  }
  blocks.push(Buffer.alloc(1024));
  return gzipSync(Buffer.concat(blocks));
}

/** A release file of `pick` with the fake program `program` inside. */
function releaseFile(pick, program) {
  if (pick.format === 'tar.gz') {
    const top = pick.exe.split('/')[0];
    return makeTarGz([
      { name: `${top}/` },
      { name: pick.exe, data: program, mode: 0o755 },
      { name: `${top}/install.sh`, data: Buffer.from('#!/bin/sh\n'), mode: 0o755 },
      { name: `${top}/share/icons/hicolor/${'x'.repeat(60)}/apps/pandite.png`, data: Buffer.from('png') },
    ]);
  }
  const top = pick.exe.split('/')[0];
  const entries = [{ name: `${top}/` }, { name: pick.exe, data: program, mode: 0o100755 }];
  if (pick.target === 'universal-apple-darwin') {
    entries.push({ name: 'pandite.app/Contents/Info.plist', data: Buffer.from('<plist/>') });
    entries.push({ name: '__MACOSX/pandite.app/._Info.plist', data: Buffer.from('xattr') });
  } else {
    entries.push({ name: `${top}/LICENSE.txt`, data: Buffer.from('MIT') });
  }
  return makeZip(entries);
}

// --------------------------------------------------------------- fakes

const empty = () => Readable.from([]);

/** Serves `files` (name -> Buffer) behind a redirect, like GitHub. */
function fakeNetwork(files, { redirectTo = 'https://release-assets.example.com/' } = {}) {
  const calls = [];
  const request = async (url) => {
    calls.push(url);
    const u = new URL(url);
    assert.equal(u.protocol, 'https:', `the launcher requested ${url}`);
    if (u.host === 'github.com') {
      const prefix = `/xls/pandite/releases/download/v${VERSION}/`;
      assert.ok(u.pathname.startsWith(prefix), `unexpected release URL ${url}`);
      return { statusCode: 302, headers: { location: redirectTo + u.pathname.slice(prefix.length) }, body: empty() };
    }
    const name = decodeURIComponent(u.pathname.slice(1));
    if (!files.has(name)) return { statusCode: 404, headers: {}, body: empty() };
    return { statusCode: 200, headers: {}, body: Readable.from([files.get(name)]) };
  };
  return { request, calls };
}

function fakeSpawn(exitCode) {
  const calls = [];
  const spawn = (exe, args, options) => {
    calls.push({ exe, args, options });
    const child = new EventEmitter();
    child.unref = () => {};
    child.kill = () => true;
    setImmediate(() => {
      child.emit('spawn');
      if (!options.detached) child.emit('exit', exitCode, null);
    });
    return child;
  };
  return { spawn, calls };
}

const quiet = () => {};

// --------------------------------------------------------------- tests

const tests = [];
const test = (name, fn) => tests.push([name, fn]);

test('every supported system maps to its release file', () => {
  const want = {
    'win32 x64': ['pandite-1.2.3-x86_64-pc-windows-msvc.zip', 'pandite-1.2.3-x86_64-pc-windows-msvc/pandite.exe'],
    'win32 arm64': ['pandite-1.2.3-aarch64-pc-windows-msvc.zip', 'pandite-1.2.3-aarch64-pc-windows-msvc/pandite.exe'],
    'darwin x64': ['pandite-1.2.3-universal-apple-darwin.zip', 'pandite.app/Contents/MacOS/pandite'],
    'darwin arm64': ['pandite-1.2.3-universal-apple-darwin.zip', 'pandite.app/Contents/MacOS/pandite'],
    'linux x64': ['pandite-1.2.3-x86_64-unknown-linux-gnu.tar.gz', 'pandite-1.2.3-x86_64-unknown-linux-gnu/pandite'],
  };
  assert.deepEqual(PLATFORMS.map((p) => `${p.platform} ${p.arch}`).sort(), Object.keys(want).sort());
  for (const p of PLATFORMS) {
    const pick = assetFor({ ...p, version: VERSION });
    assert.deepEqual([pick.asset, pick.exe], want[`${p.platform} ${p.arch}`]);
  }
  assert.equal(assetFor({ platform: 'linux', arch: 'x64', version: VERSION, glibc: '2.39' }).format, 'tar.gz');
  assert.deepEqual(allAssets(VERSION), [
    'pandite-1.2.3-x86_64-pc-windows-msvc.zip',
    'pandite-1.2.3-aarch64-pc-windows-msvc.zip',
    'pandite-1.2.3-universal-apple-darwin.zip',
    'pandite-1.2.3-x86_64-unknown-linux-gnu.tar.gz',
  ]);
});

test('unsupported systems get a clear message', async () => {
  const cases = [
    { platform: 'linux', arch: 'arm64', glibc: '2.39', says: 'Linux on arm64' },
    { platform: 'win32', arch: 'ia32', glibc: null, says: 'Windows on ia32' },
    { platform: 'freebsd', arch: 'x64', glibc: null, says: 'freebsd on x64' },
    { platform: 'darwin', arch: 'ppc64', glibc: null, says: 'macOS on ppc64' },
    { platform: 'linux', arch: 'x64', glibc: null, says: 'musl' },
    { platform: 'linux', arch: 'x64', glibc: '2.31', says: 'glibc 2.35 or later' },
  ];
  for (const c of cases) {
    const net = fakeNetwork(new Map());
    const run = fakeSpawn(0);
    await assert.rejects(
      main(['a.png'], { ...c, version: VERSION, checksums: { version: VERSION, sha256: {} }, env: { PANDITE_NPM_CACHE: tempDir() }, home: TEMP, request: net.request, spawn: run.spawn, log: quiet }),
      (error) => error instanceof LauncherError && error.message.includes(c.says) && error.message.includes('Pandite builds exist for'),
    );
    assert.equal(net.calls.length, 0);
    assert.equal(run.calls.length, 0);
  }
});

test('first run downloads, checks and unpacks; second run uses the cache', async () => {
  for (const p of PLATFORMS) {
    const pick = assetFor({ ...p, version: VERSION });
    const program = Buffer.from(`fake pandite for ${p.platform} ${p.arch}`);
    const file = releaseFile(pick, program);
    const net = fakeNetwork(new Map([[pick.asset, file]]));
    const cache = tempDir();
    const deps = {
      ...p,
      version: VERSION,
      checksums: { version: VERSION, sha256: { [pick.asset]: sha256(file) } },
      env: { PANDITE_NPM_CACHE: cache },
      home: TEMP,
      request: net.request,
      log: quiet,
    };
    const exe = path.join(cache, VERSION, pick.target, ...pick.exe.split('/'));
    // An option makes the launcher wait for the exit code on every system.
    const first = fakeSpawn(7);
    assert.deepEqual(await main(['--version'], { ...deps, spawn: first.spawn }), { code: 7, signal: null });
    assert.equal(net.calls.length, 2, `${p.platform}: one redirect and one download`);
    assert.ok(net.calls[1].startsWith('https://release-assets.example.com/'));
    assert.deepEqual(first.calls.map((c) => [c.exe, c.args, c.options.stdio]), [[exe, ['--version'], 'inherit']]);
    assert.deepEqual(readFileSync(exe), program);
    if (process.platform !== 'win32' && p.platform !== 'win32') assert.equal(statSync(exe).mode & 0o777, 0o755);
    assert.ok(!existsSync(path.join(cache, VERSION, pick.target, '__MACOSX')));
    assert.deepEqual(readdirSync(path.join(cache, VERSION)), [pick.target], 'no download folder is left behind');

    const second = fakeSpawn(3);
    assert.deepEqual(await main(['--help'], { ...deps, spawn: second.spawn }), { code: 3, signal: null });
    assert.equal(net.calls.length, 2, `${p.platform}: the second run downloads nothing`);
    assert.equal(second.calls[0].exe, exe);

    // A copy without its completion marker is not trusted and comes again.
    rmSync(path.join(cache, VERSION, pick.target, '.pandite-npm.json'));
    await main(['--help'], { ...deps, spawn: fakeSpawn(0).spawn });
    assert.equal(net.calls.length, 4, `${p.platform}: an incomplete cache downloads again`);
  }
});

test('on Windows an image opens detached, an option waits', async () => {
  assert.equal(startsDetached('win32', ['photo.jpg']), true);
  assert.equal(startsDetached('win32', []), true);
  assert.equal(startsDetached('win32', ['--background']), true);
  assert.equal(startsDetached('win32', ['-']), true);
  assert.equal(startsDetached('win32', ['--', '-odd-.png']), true);
  assert.equal(startsDetached('win32', ['--version']), false);
  assert.equal(startsDetached('win32', ['-V']), false);
  assert.equal(startsDetached('win32', ['/?']), false);
  assert.equal(startsDetached('win32', ['--smoke', 'a.png']), false);
  assert.equal(startsDetached('win32', ['--register']), false);
  assert.equal(startsDetached('linux', ['photo.jpg']), false);
  assert.equal(startsDetached('darwin', []), false);

  const p = { platform: 'win32', arch: 'x64', glibc: null };
  const pick = assetFor({ ...p, version: VERSION });
  const file = releaseFile(pick, Buffer.from('exe'));
  const run = fakeSpawn(9);
  const result = await main(['C:\\photos\\a b.jpg'], {
    ...p,
    version: VERSION,
    checksums: { version: VERSION, sha256: { [pick.asset]: sha256(file) } },
    env: { PANDITE_NPM_CACHE: tempDir() },
    home: TEMP,
    request: fakeNetwork(new Map([[pick.asset, file]])).request,
    spawn: run.spawn,
    log: quiet,
  });
  assert.deepEqual(result, { code: 0, signal: null });
  assert.deepEqual(run.calls[0].args, ['C:\\photos\\a b.jpg']);
  assert.equal(run.calls[0].options.detached, true);
  assert.equal(run.calls[0].options.stdio, 'ignore');
});

test('a file with another SHA-256 is refused and deleted', async () => {
  const p = { platform: 'linux', arch: 'x64', glibc: '2.35' };
  const pick = assetFor({ ...p, version: VERSION });
  const good = releaseFile(pick, Buffer.from('good'));
  const evil = releaseFile(pick, Buffer.from('evil'));
  const cache = tempDir();
  const run = fakeSpawn(0);
  const net = fakeNetwork(new Map([[pick.asset, evil]]));
  await assert.rejects(
    main(['a.png'], { ...p, version: VERSION, checksums: { version: VERSION, sha256: { [pick.asset]: sha256(good) } }, env: { PANDITE_NPM_CACHE: cache }, home: TEMP, request: net.request, spawn: run.spawn, log: quiet }),
    (error) => error instanceof LauncherError && /SHA-256 mismatch/.test(error.message) && error.message.includes(sha256(evil)),
  );
  assert.equal(net.calls.length, 2);
  assert.equal(run.calls.length, 0, 'the program never starts');
  assert.deepEqual(readdirSync(path.join(cache, VERSION)), [], 'nothing stays in the cache');
});

test('a redirect to plain HTTP is refused before any request', async () => {
  const p = { platform: 'win32', arch: 'arm64', glibc: null };
  const pick = assetFor({ ...p, version: VERSION });
  const file = releaseFile(pick, Buffer.from('exe'));
  const net = fakeNetwork(new Map([[pick.asset, file]]), { redirectTo: 'http://release-assets.example.com/' });
  const cache = tempDir();
  const run = fakeSpawn(0);
  await assert.rejects(
    main(['--version'], { ...p, version: VERSION, checksums: { version: VERSION, sha256: { [pick.asset]: sha256(file) } }, env: { PANDITE_NPM_CACHE: cache }, home: TEMP, request: net.request, spawn: run.spawn, log: quiet }),
    (error) => error instanceof LauncherError && /only HTTPS/.test(error.message),
  );
  assert.deepEqual(net.calls.map((u) => new URL(u).protocol), ['https:']);
  assert.equal(run.calls.length, 0);
  assert.deepEqual(readdirSync(path.join(cache, VERSION)), []);
  // The real request function is never reached for a plain HTTP address.
  await assert.rejects(download('http://127.0.0.1:9/x', path.join(TEMP, 'never')), /only HTTPS/);
});

test('redirect loops, missing files and missing checksums fail clearly', async () => {
  const loop = async (url) => ({ statusCode: 302, headers: { location: `${url}x` }, body: empty() });
  await assert.rejects(download('https://example.com/a', path.join(tempDir()), { request: loop }), /more than 5 redirects/);
  const notFound = async () => ({ statusCode: 404, headers: {}, body: empty() });
  await assert.rejects(download('https://example.com/a', path.join(tempDir()), { request: notFound }), /HTTP 404/);
  const broken = async () => {
    throw new Error('getaddrinfo ENOTFOUND example.com');
  };
  await assert.rejects(download('https://example.com/a', path.join(tempDir()), { request: broken }), /cannot download .*ENOTFOUND/);

  const p = { platform: 'darwin', arch: 'arm64', glibc: null };
  const base = { ...p, version: VERSION, env: { PANDITE_NPM_CACHE: tempDir() }, home: TEMP, request: fakeNetwork(new Map()).request, spawn: fakeSpawn(0).spawn, log: quiet };
  await assert.rejects(main([], { ...base, checksums: { version: VERSION, sha256: {} } }), /no SHA-256 for pandite-1\.2\.3-universal-apple-darwin\.zip/);
  await assert.rejects(main([], { ...base, checksums: { version: '1.2.2', sha256: {} } }), /checksums\.json is for version 1\.2\.2/);
});

test('archive entries outside their folder and links are refused', () => {
  const bad = [
    makeZip([{ name: '../evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: '/abs/evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: 'C:/evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: 'a\\..\\..\\evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: 'link', data: Buffer.from('/etc/passwd'), mode: 0o120777 }]),
  ];
  for (const zip of bad) assert.throws(() => unzip(zip, tempDir()), LauncherError);
  assert.throws(() => untar(makeTarGz([{ name: '../evil', data: Buffer.from('x') }]), tempDir()), LauncherError);
  assert.throws(() => untar(makeTarGz([{ name: 'link', type: '2' }]), tempDir()), LauncherError);
  assert.throws(() => untar(makeTarGz([{ name: 'hard', type: '1' }]), tempDir()), LauncherError);
  assert.throws(() => unzip(Buffer.from('not a zip at all, not even close to one'), tempDir()), /not a zip file/);
  // Long tar names (GNU "L" entries) unpack under their full name.
  const dest = tempDir();
  const long = `top/${'d'.repeat(120)}/file.txt`;
  assert.deepEqual(untar(makeTarGz([{ name: long, data: Buffer.from('long') }]), dest), [long]);
  assert.equal(readFileSync(path.join(dest, ...long.split('/')), 'utf8'), 'long');
});

test('checksums.json comes from SHA256SUMS.txt', () => {
  const lines = allAssets(VERSION).map((a, i) => `${String(i).repeat(64)}  ${a}`);
  lines.push(`${'A'.repeat(64)} *pandite-${VERSION}-windows.msixbundle`);
  const sums = parseSums(`${lines.join('\n')}\n`);
  assert.equal(sums.get(`pandite-${VERSION}-windows.msixbundle`), 'a'.repeat(64));
  const made = makeChecksums(VERSION, sums);
  assert.equal(made.version, VERSION);
  assert.deepEqual(Object.keys(made.sha256), allAssets(VERSION));
  assert.equal(made.sha256[`pandite-${VERSION}-universal-apple-darwin.zip`], '2'.repeat(64));
  const missing = parseSums(lines.slice(1).join('\n'));
  assert.throws(() => makeChecksums(VERSION, missing), /no line for pandite-1\.2\.3-x86_64-pc-windows-msvc\.zip/);
  assert.throws(() => parseSums('abc  file\n'), /not a sha256sum line/);
});

let failed = false;
try {
  for (const [name, fn] of tests) {
    try {
      await fn();
    } catch (error) {
      failed = true;
      console.error(`STEP FAILED: ${name}\n${error.stack ?? error}`);
    }
  }
} finally {
  rmSync(TEMP, { recursive: true, force: true });
}
if (failed) process.exit(1);
console.log('NPM LAUNCHER VERIFIED');
