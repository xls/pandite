#!/usr/bin/env node
// Offline test of the npm installer. No network and no real installer: a
// fake fetch serves the GitHub API and the release files from memory, a
// fake process runner records every program start and imitates reg.exe,
// the setup program, its uninstaller, install.sh and hdiutil, and every
// folder (home, data, temporary, /Applications) is in a temporary folder.
//
// Usage: node packaging/npm/test.mjs
//
// Prints "NPM INSTALLER VERIFIED" as its last line on success and exits
// with a nonzero code otherwise.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { deflateRawSync, gzipSync } from 'node:zlib';
import { untar, unzip } from './lib/archive.js';
import { INSTALLER_VERSION, defaultDeps, main, parseArgs } from './lib/cli.js';
import { InstallerError } from './lib/errors.js';
import { parseSums } from './lib/github.js';
import {
  DEFAULT_QUESTION,
  DESKTOP_FILE,
  FLOAT_QUESTION,
  HDIUTIL,
  HYPRLAND_RULE,
  MARKER,
  SETUP_ARGS,
  SH,
  UNINSTALL_ARGS,
  UNINSTALL_KEY,
  XDG_MIME,
  parseRegFile,
  regExe,
} from './lib/install.js';
import { download } from './lib/net.js';
import { PLATFORMS, chooseAsset, dataDir } from './lib/platform.js';
import { compareVersions, normalizeVersion } from './lib/version.js';

const NOW = Date.UTC(2026, 10, 14, 12, 0, 0);
const TEMP = mkdtempSync(path.join(os.tmpdir(), 'pandite-installer-test-'));
let tempCount = 0;
const tempDir = () => {
  tempCount += 1;
  const dir = path.join(TEMP, String(tempCount));
  mkdirSync(dir, { recursive: true });
  return dir;
};
const sha256 = (buf) => createHash('sha256').update(buf).digest('hex');
const API = 'https://api.github.com/repos/xls/pandite';

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
    const dir = e.name.endsWith('/');
    const packed = dir ? data : deflateRawSync(data);
    const method = dir ? 0 : 8;
    const mode = e.mode ?? (dir ? 0o040755 : 0o100644);
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

// --------------------------------------------------------- release files

const setupExe = (version, arch) => Buffer.from(`MZ fake setup program of Pandite ${version} for ${arch}`);

function macZip(version) {
  return makeZip([
    { name: 'Pandite.app/' },
    { name: 'Pandite.app/Contents/' },
    { name: 'Pandite.app/Contents/MacOS/' },
    { name: 'Pandite.app/Contents/MacOS/pandite', data: Buffer.from(`pandite ${version} universal`), mode: 0o100755 },
    { name: 'Pandite.app/Contents/Info.plist', data: Buffer.from(`<plist>${version}</plist>`) },
    { name: '__MACOSX/Pandite.app/._Info.plist', data: Buffer.from('xattr') },
  ]);
}

function linuxTarball(version) {
  const top = `pandite-${version}-x86_64-unknown-linux-gnu`;
  return makeTarGz([
    { name: `${top}/` },
    { name: `${top}/pandite`, data: Buffer.from(`pandite ${version} linux`), mode: 0o755 },
    { name: `${top}/install.sh`, data: Buffer.from('#!/bin/sh\necho fake\n'), mode: 0o755 },
    { name: `${top}/share/applications/pandite.desktop`, data: Buffer.from('[Desktop Entry]\nName=Pandite\nMimeType=image/jpeg;image/png;image/tiff;image/x-sony-arw;\n') },
    { name: `${top}/share/icons/hicolor/48x48/apps/pandite.png`, data: Buffer.from('png48') },
    { name: `${top}/share/icons/hicolor/256x256/apps/pandite.png`, data: Buffer.from('png256') },
  ]);
}

/**
 * The files of a release under the names the build scripts write, with
 * SHA256SUMS.txt. `omit` leaves files out, `unlist` leaves them out of
 * SHA256SUMS.txt, `corrupt` changes them after the sums were made.
 */
function releaseFiles(version, { omit = [], unlist = [], corrupt = [], sums = true } = {}) {
  const files = new Map([
    [`pandite-${version}-x86_64-pc-windows-msvc-setup.exe`, setupExe(version, 'x64')],
    [`pandite-${version}-aarch64-pc-windows-msvc-setup.exe`, setupExe(version, 'arm64')],
    [`pandite-${version}-x86_64-pc-windows-msvc.zip`, Buffer.from('portable zip')],
    [`pandite-${version}-aarch64-pc-windows-msvc.zip`, Buffer.from('portable zip')],
    [`pandite-${version}-universal-apple-darwin.zip`, macZip(version)],
    [`pandite-${version}-universal-apple-darwin.dmg`, Buffer.from(`dmg ${version}`)],
    [`pandite-${version}-x86_64-unknown-linux-gnu.tar.gz`, linuxTarball(version)],
    [`pandite_${version}_amd64.deb`, Buffer.from('deb')],
  ]);
  for (const name of omit) files.delete(name);
  if (sums) {
    const lines = [...files].filter(([name]) => !unlist.includes(name)).map(([name, data]) => `${sha256(data)}  ${name}`);
    files.set('SHA256SUMS.txt', Buffer.from(`${lines.join('\n')}\n`));
  }
  for (const name of corrupt) files.set(name, Buffer.concat([files.get(name), Buffer.from('evil')]));
  return files;
}

const rel = (version, options = {}) => ({ version, prerelease: false, files: releaseFiles(version, options) });

// ----------------------------------------------------------- fake GitHub

const json = (status, body, headers = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

/**
 * A fetch function that answers like GitHub: the API, release downloads
 * that redirect to a file host, and the file host. It fails on any request
 * that is not HTTPS or does not ask for manual redirects.
 */
function fakeGitHub(releases, { latest = null, fileHost = 'https://objects.example.com/', rateLimit = false, apiStatus = null } = {}) {
  const calls = [];
  const byTag = new Map(releases.map((r) => [r.tag ?? `v${r.version}`, r]));
  const releaseJson = (r) => ({
    tag_name: r.tag ?? `v${r.version}`,
    draft: Boolean(r.draft),
    prerelease: Boolean(r.prerelease),
    assets: [...(r.files ?? [])].map(([name, data]) => ({
      name,
      size: data.length,
      browser_download_url: `https://github.com/xls/pandite/releases/download/${r.tag ?? `v${r.version}`}/${encodeURIComponent(name)}`,
    })),
  });
  async function fetch(url, init = {}) {
    calls.push({ url, headers: { ...init.headers }, redirect: init.redirect });
    const u = new URL(url);
    assert.equal(u.protocol, 'https:', `the installer requested ${url}`);
    assert.equal(init.redirect, 'manual', 'the installer follows redirects itself');
    if (u.host === 'api.github.com') {
      if (rateLimit) {
        return json(403, { message: 'API rate limit exceeded' }, { 'x-ratelimit-remaining': '0', 'x-ratelimit-reset': String(NOW / 1000 + 25 * 60) });
      }
      if (apiStatus) return json(apiStatus, { message: 'Server Error' });
      if (u.pathname === '/repos/xls/pandite/releases/latest') {
        const r = latest && byTag.get(`v${latest}`);
        return r ? json(200, releaseJson(r)) : json(404, { message: 'Not Found' });
      }
      const tag = /^\/repos\/xls\/pandite\/releases\/tags\/([^/]+)$/.exec(u.pathname);
      if (tag) {
        const r = byTag.get(decodeURIComponent(tag[1]));
        return r && !r.draft ? json(200, releaseJson(r)) : json(404, { message: 'Not Found' });
      }
      if (u.pathname === '/repos/xls/pandite/releases') return json(200, releases.map(releaseJson));
      return json(404, { message: 'Not Found' });
    }
    if (u.host === 'github.com') {
      const m = /^\/xls\/pandite\/releases\/download\/([^/]+)\/([^/]+)$/.exec(u.pathname);
      if (!m) return new Response('Not Found', { status: 404 });
      return new Response(null, { status: 302, headers: { location: `${fileHost}${m[1]}/${m[2]}` } });
    }
    if (u.host === 'objects.example.com') {
      const m = /^\/([^/]+)\/([^/]+)$/.exec(u.pathname);
      const data = m && byTag.get(m[1])?.files?.get(decodeURIComponent(m[2]));
      return data ? new Response(data, { status: 200 }) : new Response('Not Found', { status: 404 });
    }
    throw new Error(`unexpected request ${url}`);
  }
  return {
    fetch,
    calls,
    apiCalls: () => calls.filter((c) => c.url.startsWith('https://api.github.com/')).map((c) => c.url),
    downloads: () =>
      calls.filter((c) => new URL(c.url).host === 'objects.example.com').map((c) => decodeURIComponent(c.url.split('/').pop())),
  };
}

// ----------------------------------------------------------- fake system

function regFile(key, values) {
  const esc = (s) => String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  const hive = { HKCU: 'HKEY_CURRENT_USER', HKLM: 'HKEY_LOCAL_MACHINE' };
  const [root, ...rest] = key.split('\\');
  const lines = ['Windows Registry Editor Version 5.00', '', `[${hive[root]}\\${rest.join('\\')}]`];
  for (const [name, value] of Object.entries(values)) lines.push(`"${esc(name)}"="${esc(value)}"`);
  lines.push('"EstimatedSize"=dword:00000400', '');
  return Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(lines.join('\r\n'), 'utf16le')]);
}

/**
 * A system in a temporary folder. `run` imitates the programs that the
 * installer starts and fails on any other program.
 */
function fakeSystem(platform, arch = 'x64', { glibc = platform === 'linux' ? '2.39' : null } = {}) {
  const root = tempDir();
  const home = path.join(root, 'home');
  const tmp = path.join(root, 'tmp');
  const systemApplications = path.join(root, 'Applications');
  for (const dir of [home, tmp, systemApplications]) mkdirSync(dir, { recursive: true });
  const env = { HOME: home, LOCALAPPDATA: path.join(home, 'AppData', 'Local'), SystemRoot: path.join(root, 'Windows') };
  const installDir = path.join(env.LOCALAPPDATA, 'Programs', 'Pandite');
  const uninstaller = path.join(installDir, 'unins000.exe');
  const registry = new Map();
  const calls = [];
  const out = [];
  const err = [];
  const state = { setupExit: 0, uninstallerExit: 0, entryLingers: 0, lingering: null, setupFiles: [], sleeps: 0, xdgMimeExit: 0 };

  async function run(command, args, options = {}) {
    calls.push({ command, args: [...args], env: options.env, capture: Boolean(options.capture) });
    const ok = (stdout = '') => ({ code: 0, signal: null, stdout, stderr: '' });
    if (command === regExe(env)) {
      assert.equal(platform, 'win32');
      assert.equal(args[0], 'export');
      assert.equal(args[3], '/y');
      const [hive, ...rest] = args[1].split('\\');
      if (state.lingering === hive) {
        if (state.entryLingers > 0) state.entryLingers -= 1;
        else {
          registry.delete(hive);
          state.lingering = null;
        }
      }
      const values = registry.get(hive);
      if (!values || rest.join('\\') !== UNINSTALL_KEY) {
        return { code: 1, signal: null, stdout: '', stderr: 'ERROR: The system was unable to find the specified registry key or value.' };
      }
      writeFileSync(args[2], regFile(args[1], values));
      return ok();
    }
    const setup = /^pandite-(.+)-(x86_64|aarch64)-pc-windows-msvc-setup\.exe$/.exec(path.basename(command));
    if (setup) {
      assert.equal(platform, 'win32');
      assert.ok(command.startsWith(tmp + path.sep), 'the setup program runs from the temporary folder');
      state.setupFiles.push(readFileSync(command));
      if (state.setupExit) return { code: state.setupExit, signal: null, stdout: '', stderr: '' };
      mkdirSync(installDir, { recursive: true });
      writeFileSync(path.join(installDir, 'pandite.exe'), `pandite ${setup[1]}`);
      writeFileSync(uninstaller, 'uninstaller');
      registry.set('HKCU', {
        DisplayName: 'Pandite',
        DisplayVersion: setup[1],
        InstallLocation: `${installDir}\\`,
        UninstallString: `"${uninstaller}"`,
        QuietUninstallString: `"${uninstaller}" /SILENT`,
      });
      return ok();
    }
    if (command === uninstaller) {
      if (state.uninstallerExit) return { code: state.uninstallerExit, signal: null, stdout: '', stderr: '' };
      rmSync(installDir, { recursive: true, force: true });
      state.lingering = 'HKCU';
      return ok();
    }
    if (command === SH) {
      assert.equal(platform, 'linux');
      const [script, flag] = args;
      const prefix = options.env?.PREFIX;
      assert.ok(prefix, 'install.sh gets PREFIX');
      const bin = path.join(prefix, 'bin', 'pandite');
      if (flag === '--uninstall') {
        assert.ok(existsSync(path.join(path.dirname(script), 'share', 'icons', 'hicolor')), 'the kept install.sh has the icon folders');
        rmSync(bin, { force: true });
        return ok(`pandite removed from ${prefix}\n`);
      }
      assert.equal(args.length, 1);
      mkdirSync(path.dirname(bin), { recursive: true });
      writeFileSync(bin, readFileSync(path.join(path.dirname(script), 'pandite')));
      return ok(`pandite installed in ${bin}\n`);
    }
    if (command === XDG_MIME) {
      assert.equal(platform, 'linux');
      return { code: state.xdgMimeExit, signal: null, stdout: '', stderr: state.xdgMimeExit ? 'xdg-mime: no desktop\n' : '' };
    }
    if (command === HDIUTIL) {
      assert.equal(platform, 'darwin');
      if (args[0] === 'attach') {
        const mount = args[args.indexOf('-mountpoint') + 1];
        const version = /^dmg (.+)$/.exec(readFileSync(args[args.length - 1], 'utf8'))[1];
        const exe = path.join(mount, 'Pandite.app', 'Contents', 'MacOS', 'pandite');
        mkdirSync(path.dirname(exe), { recursive: true });
        writeFileSync(exe, `pandite ${version} from dmg`);
        return ok();
      }
      if (args[0] === 'detach') return ok();
    }
    throw new Error(`unexpected program start: ${command} ${args.join(' ')}`);
  }

  const deps = {
    platform,
    arch,
    glibc,
    env,
    home,
    tmpdir: tmp,
    systemApplications,
    fetch: () => {
      throw new Error('no fetch for this test');
    },
    run,
    sleep: async () => {
      state.sleeps += 1;
    },
    // No one answers questions, as in a pipe. A test sets its own answers.
    ask: null,
    now: () => NOW,
    out: (line) => out.push(line),
    err: (line) => err.push(line),
  };
  for (const key of Object.keys(defaultDeps())) assert.ok(key in deps, `the fake system replaces ${key}`);
  return { home, tmp, env, systemApplications, installDir, uninstaller, registry, calls, state, deps, data: dataDir({ platform, env, home }), out, err };
}

/** Run the installer with `argv`: { code, out, err } of this run. */
async function install(sys, gh, argv) {
  sys.out.length = 0;
  sys.err.length = 0;
  const code = await main(argv, { ...sys.deps, fetch: gh.fetch });
  return { code, out: [...sys.out], err: [...sys.err] };
}

const programs = (sys) => sys.calls.filter((c) => c.command !== regExe(sys.env));
const marker = (sys) => JSON.parse(readFileSync(path.join(sys.data, MARKER), 'utf8'));
const noTempLeft = (sys) => assert.deepEqual(readdirSync(sys.tmp), [], 'no temporary folder stays behind');

// ----------------------------------------------------------------- tests

const tests = [];
const test = (name, fn) => tests.push([name, fn]);

test('the latest release: resolve, download, check and install (Linux)', async () => {
  const gh = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
  const sys = fakeSystem('linux');
  const r = await install(sys, gh, []);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.deepEqual(gh.apiCalls(), [`${API}/releases/latest`]);
  assert.deepEqual(gh.downloads(), ['SHA256SUMS.txt', 'pandite-0.2.0-x86_64-unknown-linux-gnu.tar.gz']);
  const bin = path.join(sys.home, '.local', 'bin', 'pandite');
  assert.equal(readFileSync(bin, 'utf8'), 'pandite 0.2.0 linux');
  assert.deepEqual(r.out, [`Pandite 0.2.0 installed in ${bin}. Start it from the application menu or with: ${bin}`]);
  assert.equal(marker(sys).version, '0.2.0');
  assert.equal(marker(sys).sha256, sha256(releaseFiles('0.2.0').get('pandite-0.2.0-x86_64-unknown-linux-gnu.tar.gz')));
  noTempLeft(sys);
  for (const c of gh.calls) {
    assert.equal(c.headers['user-agent'], `pandite-installer/${INSTALLER_VERSION}`, 'a generic User-Agent');
    assert.ok(!Object.keys(c.headers).some((h) => /authorization|cookie/i.test(h)), 'no credentials');
  }
});

test('the Linux install runs install.sh with PREFIX and keeps a copy for the uninstall', async () => {
  const gh = fakeGitHub([rel('0.2.0')], { latest: '0.2.0' });
  const sys = fakeSystem('linux');
  assert.equal((await install(sys, gh, [])).code, 0);
  const sh = programs(sys);
  assert.equal(sh.length, 1);
  assert.equal(sh[0].command, SH);
  const script = sh[0].args[0];
  assert.deepEqual(sh[0].args, [script]);
  assert.ok(script.startsWith(sys.tmp + path.sep));
  assert.equal(path.relative(sys.tmp, script).split(path.sep).slice(1).join('/'), 'unpacked/pandite-0.2.0-x86_64-unknown-linux-gnu/install.sh');
  assert.equal(sh[0].env.PREFIX, path.join(sys.home, '.local'));
  assert.equal(sh[0].env.HOME, sys.home, 'install.sh gets the environment');
  const keep = path.join(sys.data, 'linux');
  assert.equal(readFileSync(path.join(keep, 'install.sh'), 'utf8'), '#!/bin/sh\necho fake\n');
  assert.equal(readFileSync(path.join(keep, 'share', 'icons', 'hicolor', '256x256', 'apps', 'pandite.png'), 'utf8'), 'png256');
  assert.equal(marker(sys).prefix, path.join(sys.home, '.local'));
});

test('a named version, with and without "v"', async () => {
  for (const argv of [['-v', '0.1.0'], ['-v', 'v0.1.0'], ['install', '--version', 'v0.1.0'], ['install', '-v=0.1.0'], ['--version=V0.1.0']]) {
    const gh = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
    const sys = fakeSystem('win32', 'x64');
    const r = await install(sys, gh, argv);
    assert.equal(r.code, 0, `${argv.join(' ')}: ${r.err.join('\n')}`);
    assert.deepEqual(gh.apiCalls(), [`${API}/releases/tags/v0.1.0`], argv.join(' '));
    assert.deepEqual(gh.downloads(), ['SHA256SUMS.txt', 'pandite-0.1.0-x86_64-pc-windows-msvc-setup.exe']);
  }
  assert.equal(normalizeVersion('1.0'), '1.0.0');
  assert.equal(normalizeVersion('v2'), '2.0.0');
  assert.equal(normalizeVersion('v1.0.0-rc.1'), '1.0.0-rc.1');
  const gh = fakeGitHub([rel('0.1.0')], { latest: '0.1.0' });
  const sys = fakeSystem('linux');
  const missing = await install(sys, gh, ['-v', '9.9.9']);
  assert.equal(missing.code, 1);
  assert.deepEqual(missing.err, [
    'pandite-installer: no Pandite release v9.9.9 found at https://github.com/xls/pandite/releases. Run "npx pandite --list" for the versions.',
  ]);
  const bad = await install(sys, gh, ['-v', 'latest']);
  assert.equal(bad.code, 2);
  assert.match(bad.err[0], /"latest" is not a version such as 0\.1\.0 or v0\.1\.0/);
});

test('the release file of every supported system', async () => {
  const want = {
    'win32 x64': ['pandite-1.2.3-x86_64-pc-windows-msvc-setup.exe', 'setup'],
    'win32 arm64': ['pandite-1.2.3-aarch64-pc-windows-msvc-setup.exe', 'setup'],
    'darwin x64': ['pandite-1.2.3-universal-apple-darwin.zip', 'app-zip'],
    'darwin arm64': ['pandite-1.2.3-universal-apple-darwin.zip', 'app-zip'],
    'linux x64': ['pandite-1.2.3-x86_64-unknown-linux-gnu.tar.gz', 'tarball'],
  };
  assert.deepEqual(PLATFORMS.map((p) => `${p.platform} ${p.arch}`).sort(), Object.keys(want).sort());
  const all = [...releaseFiles('1.2.3').keys()];
  for (const p of PLATFORMS) {
    const pick = chooseAsset({ ...p, glibc: '2.35', version: '1.2.3', available: all });
    assert.deepEqual([pick.name, pick.kind], want[`${p.platform} ${p.arch}`]);
  }
  // Without the zip of the app, macOS takes the disk image.
  const noZip = all.filter((n) => !n.endsWith('-apple-darwin.zip'));
  assert.deepEqual(chooseAsset({ platform: 'darwin', arch: 'arm64', version: '1.2.3', available: noZip }), {
    name: 'pandite-1.2.3-universal-apple-darwin.dmg',
    kind: 'dmg',
  });
  // A release with the short names: every system takes those.
  const short = [
    'pandite-1.2.3-x86_64-windows-setup.exe',
    'pandite-1.2.3-aarch64-windows-setup.exe',
    'pandite-1.2.3-universal-macos.dmg',
    'pandite-1.2.3-x86_64-linux.tar.gz',
    'SHA256SUMS.txt',
  ];
  for (const [system, name, kind] of [
    [{ platform: 'win32', arch: 'x64' }, short[0], 'setup'],
    [{ platform: 'win32', arch: 'arm64' }, short[1], 'setup'],
    [{ platform: 'darwin', arch: 'arm64' }, short[2], 'dmg'],
    [{ platform: 'linux', arch: 'x64', glibc: '2.39' }, short[3], 'tarball'],
  ]) {
    assert.deepEqual(chooseAsset({ ...system, version: '1.2.3', available: short }), { name, kind });
  }
  assert.throws(
    () => chooseAsset({ platform: 'linux', arch: 'x64', glibc: '2.39', version: '1.2.3', available: ['SHA256SUMS.txt'] }),
    /release v1\.2\.3 has no pandite-1\.2\.3-x86_64-linux\.tar\.gz or pandite-1\.2\.3-x86_64-unknown-linux-gnu\.tar\.gz for Linux x64/,
  );
  // End to end: each system downloads its own file.
  for (const p of PLATFORMS) {
    const gh = fakeGitHub([rel('1.2.3')], { latest: '1.2.3' });
    const sys = fakeSystem(p.platform, p.arch);
    const r = await install(sys, gh, []);
    assert.equal(r.code, 0, `${p.platform} ${p.arch}: ${r.err.join('\n')}`);
    assert.deepEqual(gh.downloads(), ['SHA256SUMS.txt', want[`${p.platform} ${p.arch}`][0]]);
    noTempLeft(sys);
  }
});

test('an unsupported system gets a clear message, no request and no program start', async () => {
  const cases = [
    { platform: 'linux', arch: 'arm64', glibc: '2.39', says: 'Linux arm64 is not supported.' },
    { platform: 'win32', arch: 'ia32', says: 'Windows ia32 is not supported.' },
    { platform: 'darwin', arch: 'ppc64', says: 'macOS ppc64 is not supported.' },
    { platform: 'freebsd', arch: 'x64', says: 'freebsd x64 is not supported.' },
    { platform: 'linux', arch: 'x64', glibc: null, says: 'Linux without glibc (for example with musl) is not supported.' },
    { platform: 'linux', arch: 'x64', glibc: '2.31', says: 'Linux with glibc 2.31 is not supported.' },
  ];
  for (const c of cases) {
    for (const argv of [[], ['update'], ['uninstall'], ['--dry-run']]) {
      const gh = fakeGitHub([rel('0.2.0')], { latest: '0.2.0' });
      const sys = fakeSystem(c.platform, c.arch, { glibc: c.glibc ?? null });
      const r = await install(sys, gh, argv);
      assert.equal(r.code, 1, `${c.says} ${argv.join(' ')}`);
      assert.equal(r.err.length, 1);
      assert.ok(r.err[0].startsWith(`pandite-installer: ${c.says} Pandite releases exist for Windows x64 and ARM64`), r.err[0]);
      assert.equal(gh.calls.length, 0);
      assert.equal(sys.calls.length, 0);
    }
  }
});

test('a download with another SHA-256 is refused and deleted', async () => {
  for (const [platform, asset] of [
    ['linux', 'pandite-0.2.0-x86_64-unknown-linux-gnu.tar.gz'],
    ['win32', 'pandite-0.2.0-x86_64-pc-windows-msvc-setup.exe'],
    ['darwin', 'pandite-0.2.0-universal-apple-darwin.zip'],
  ]) {
    const gh = fakeGitHub([rel('0.2.0', { corrupt: [asset] })], { latest: '0.2.0' });
    const sys = fakeSystem(platform);
    const r = await install(sys, gh, []);
    assert.equal(r.code, 1);
    const good = sha256(releaseFiles('0.2.0').get(asset));
    const bad = sha256(Buffer.concat([releaseFiles('0.2.0').get(asset), Buffer.from('evil')]));
    assert.equal(
      r.err.at(-1),
      `pandite-installer: SHA-256 mismatch for ${asset}: SHA256SUMS.txt lists ${good}, the download has ${bad}. The download was deleted. Nothing was installed.`,
    );
    assert.deepEqual(gh.downloads(), ['SHA256SUMS.txt', asset], 'the file was downloaded');
    assert.deepEqual(programs(sys), [], 'no installer ran');
    assert.equal(sys.state.setupFiles.length, 0);
    assert.ok(!existsSync(path.join(sys.data, MARKER)));
    assert.ok(!existsSync(path.join(sys.home, 'Applications', 'Pandite.app')));
    noTempLeft(sys);
  }
});

test('a file that SHA256SUMS.txt does not list, or a release without SHA256SUMS.txt, is refused', async () => {
  const asset = 'pandite-0.2.0-aarch64-pc-windows-msvc-setup.exe';
  let gh = fakeGitHub([rel('0.2.0', { unlist: [asset] })], { latest: '0.2.0' });
  let sys = fakeSystem('win32', 'arm64');
  let r = await install(sys, gh, []);
  assert.equal(r.code, 1);
  assert.equal(r.err.at(-1), `pandite-installer: SHA256SUMS.txt of release v0.2.0 does not list ${asset}. Nothing was installed.`);
  assert.deepEqual(gh.downloads(), ['SHA256SUMS.txt'], 'the unlisted file is not downloaded');
  assert.equal(sys.state.setupFiles.length, 0);
  noTempLeft(sys);

  gh = fakeGitHub([rel('0.2.0', { sums: false })], { latest: '0.2.0' });
  sys = fakeSystem('linux');
  r = await install(sys, gh, []);
  assert.equal(r.code, 1);
  assert.deepEqual(r.err, ['pandite-installer: release v0.2.0 has no SHA256SUMS.txt, so its files cannot be checked. Nothing was installed.']);
  assert.deepEqual(gh.downloads(), []);
  assert.deepEqual(programs(sys), []);
});

test('redirects go to HTTPS addresses only', async () => {
  // A release download that redirects to plain HTTP.
  const gh = fakeGitHub([rel('0.2.0')], { latest: '0.2.0', fileHost: 'http://objects.example.com/' });
  const sys = fakeSystem('linux');
  const r = await install(sys, gh, []);
  assert.equal(r.code, 1);
  assert.equal(
    r.err.at(-1),
    'pandite-installer: refusing the redirect to http://objects.example.com/v0.2.0/SHA256SUMS.txt: only HTTPS addresses are allowed',
  );
  assert.ok(gh.calls.every((c) => c.url.startsWith('https://')), 'no plain HTTP request');
  assert.deepEqual(programs(sys), []);
  noTempLeft(sys);

  // An API answer that redirects to plain HTTP.
  const calls = [];
  const apiToHttp = async (url) => {
    calls.push(url);
    return new Response(null, { status: 301, headers: { location: 'http://api.github.com/repos/xls/pandite/releases/latest' } });
  };
  const sys2 = fakeSystem('linux');
  const r2 = await install(sys2, { fetch: apiToHttp }, []);
  assert.equal(r2.code, 1);
  assert.match(r2.err[0], /refusing the redirect to http:\/\/api\.github\.com\/.*: only HTTPS addresses are allowed$/);
  assert.deepEqual(calls, [`${API}/releases/latest`]);

  // A plain HTTP address is refused before any request; loops end.
  const net = { fetch: async () => assert.fail('no request for a plain HTTP address'), userAgent: 'test' };
  await assert.rejects(download('http://127.0.0.1:9/x', path.join(tempDir(), 'x'), net), /refusing http:\/\/127\.0\.0\.1:9\/x: only HTTPS/);
  const loop = { fetch: async (url) => new Response(null, { status: 302, headers: { location: `${url}x` } }), userAgent: 'test' };
  await assert.rejects(download('https://example.com/a', path.join(tempDir(), 'x'), loop), /more than 5 redirects/);
});

test('the same version is skipped unless --force', async () => {
  const gh = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
  const sys = fakeSystem('linux');
  assert.equal((await install(sys, gh, [])).code, 0);
  const bin = path.join(sys.home, '.local', 'bin', 'pandite');
  const again = await install(sys, gh, []);
  assert.equal(again.code, 0);
  assert.deepEqual(again.out, [`Pandite 0.2.0 is already installed in ${bin}. To install it again, add --force.`]);
  assert.equal(gh.downloads().length, 2, 'no second download');
  const calls = gh.calls.length;
  const named = await install(sys, gh, ['-v', 'v0.2.0']);
  assert.equal(named.code, 0);
  assert.deepEqual(named.out, again.out);
  assert.equal(gh.calls.length, calls, 'a named version that is installed needs no request');
  const forced = await install(sys, gh, ['--force']);
  assert.equal(forced.code, 0);
  assert.equal(gh.downloads().length, 4);
  assert.equal(programs(sys).length, 2, 'install.sh ran again');

  // Windows: the uninstall entry of the setup program tells the version.
  const ghw = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
  const win = fakeSystem('win32', 'x64');
  assert.equal((await install(win, ghw, ['-v', '0.1.0'])).code, 0);
  const skip = await install(win, ghw, ['-v', '0.1.0']);
  assert.deepEqual(skip.out, [`Pandite 0.1.0 is already installed in ${win.installDir}. To install it again, add --force.`]);
  assert.equal(win.state.setupFiles.length, 1);
  assert.equal((await install(win, ghw, ['-v', '0.1.0', '--force'])).code, 0);
  assert.equal(win.state.setupFiles.length, 2);
  // A plain install never goes back to an older latest release.
  const newer = fakeGitHub([rel('0.3.0-rc.1'), rel('0.2.0')], { latest: '0.2.0' });
  assert.equal((await install(win, newer, ['-v', '0.3.0-rc.1'])).code, 0);
  const keep = await install(win, newer, []);
  assert.equal(keep.code, 0);
  assert.match(keep.out[0], /^Pandite 0\.3\.0-rc\.1 is installed in .*\. It is newer than the latest release 0\.2\.0\./);
  assert.equal(win.state.setupFiles.length, 3);
});

test('update: newer installs, same downloads nothing, not installed installs', async () => {
  // Not installed: update installs the latest release.
  const gh = fakeGitHub([rel('0.10.0'), rel('0.9.0')], { latest: '0.10.0' });
  const sys = fakeSystem('linux');
  let r = await install(sys, gh, ['update']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.equal(marker(sys).version, '0.10.0');
  // Same version: "already up to date", one API request, no download.
  const before = gh.calls.length;
  r = await install(sys, gh, ['update']);
  assert.equal(r.code, 0);
  assert.deepEqual(r.out, ['Pandite 0.10.0 is already up to date (latest release: 0.10.0).']);
  assert.deepEqual(gh.calls.slice(before).map((c) => c.url), [`${API}/releases/latest`]);
  // Older version: 0.9.0 is older than 0.10.0 (not a text comparison).
  assert.equal((await install(sys, gh, ['-v', '0.9.0'])).code, 0);
  assert.equal(marker(sys).version, '0.9.0');
  r = await install(sys, gh, ['update']);
  assert.equal(r.code, 0);
  assert.equal(marker(sys).version, '0.10.0');
  assert.equal(readFileSync(path.join(sys.home, '.local', 'bin', 'pandite'), 'utf8'), 'pandite 0.10.0 linux');
  // A pre-release is older than its final release.
  const final = fakeGitHub([rel('1.0.0'), rel('1.0.0-rc.1')], { latest: '1.0.0' });
  const sys2 = fakeSystem('darwin', 'arm64');
  assert.equal((await install(sys2, final, ['-v', '1.0.0-rc.1'])).code, 0);
  r = await install(sys2, final, ['update']);
  assert.equal(r.code, 0);
  assert.equal(marker(sys2).version, '1.0.0');
  // An installed version newer than the latest release is up to date.
  const older = fakeGitHub([rel('0.10.0'), rel('0.9.0')], { latest: '0.9.0' });
  const sys3 = fakeSystem('linux');
  assert.equal((await install(sys3, older, ['-v', '0.10.0'])).code, 0);
  r = await install(sys3, older, ['update']);
  assert.deepEqual(r.out, ['Pandite 0.10.0 is already up to date (latest release: 0.9.0).']);
  // Windows: a setup program run by hand counts as installed (no marker).
  const ghw = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
  const win = fakeSystem('win32', 'arm64');
  win.registry.set('HKCU', { DisplayVersion: '0.1.0', InstallLocation: `${win.installDir}\\`, UninstallString: `"${win.uninstaller}"` });
  r = await install(win, ghw, ['update']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.ok(r.err.includes('pandite-installer: updating Pandite 0.1.0 to 0.2.0'));
  assert.deepEqual(ghw.downloads(), ['SHA256SUMS.txt', 'pandite-0.2.0-aarch64-pc-windows-msvc-setup.exe']);
  r = await install(win, ghw, ['update']);
  assert.deepEqual(r.out, ['Pandite 0.2.0 is already up to date (latest release: 0.2.0).']);

  // Version order (https://semver.org, section 11).
  const order = ['0.9.0', '0.10.0', '1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-alpha.beta', '1.0.0-beta', '1.0.0-beta.2', '1.0.0-beta.11', '1.0.0-rc.1', '1.0.0', '1.0.1', '1.10.0', '2.0.0'];
  for (let i = 0; i < order.length; i += 1) {
    for (let j = 0; j < order.length; j += 1) {
      assert.equal(compareVersions(order[i], order[j]), Math.sign(i - j), `${order[i]} vs ${order[j]}`);
    }
  }
  assert.equal(compareVersions('1.0.0+build.5', '1.0.0'), 0, 'build metadata does not count');
});

test('Linux: the default viewer and the Hyprland floating window', async () => {
  const types = ['image/jpeg', 'image/png', 'image/tiff', 'image/x-sony-arw'];
  const rule = `${HYPRLAND_RULE}\n`;
  // A system with xdg-mime in PATH and a Hyprland configuration.
  const desktop = (hyprland = 'local o = require("omarchy")\n') => {
    const sys = fakeSystem('linux');
    const bin = path.join(sys.home, 'bin');
    mkdirSync(bin, { recursive: true });
    writeFileSync(path.join(bin, XDG_MIME), '');
    sys.env.PATH = bin;
    const hypr = path.join(sys.home, '.config', 'hypr', 'hyprland.lua');
    if (hyprland !== null) {
      mkdirSync(path.dirname(hypr), { recursive: true });
      writeFileSync(hypr, hyprland);
    }
    const asked = [];
    const answer = (answers) => {
      sys.deps.ask = async (question) => {
        asked.push(question);
        return answers.shift() ?? '';
      };
    };
    return { sys, hypr, asked, answer, mime: () => sys.calls.filter((c) => c.command === XDG_MIME) };
  };
  const gh = () => fakeGitHub([rel('0.2.0')], { latest: '0.2.0' });

  // A first install asks both questions. Yes to both: xdg-mime gets every
  // MIME type of the desktop entry, and the rule goes to the end of the file.
  let d = desktop();
  d.answer(['y', 'Yes']);
  let r = await install(d.sys, gh(), []);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.deepEqual(d.asked, [DEFAULT_QUESTION, FLOAT_QUESTION]);
  assert.deepEqual(d.mime().map((c) => c.args), [['default', DESKTOP_FILE, ...types]]);
  assert.equal(readFileSync(d.hypr, 'utf8'), `local o = require("omarchy")\n${rule}`);
  assert.ok(r.out.includes('Pandite is the default viewer for 4 image formats.'));
  assert.ok(r.out.includes(`Added the floating window rule for Pandite to ${d.hypr}.`));

  // No, an empty line and anything else leave the system as it was.
  for (const answers of [['n', 'no'], ['', ''], ['maybe', 'x']]) {
    d = desktop();
    d.answer(answers);
    assert.equal((await install(d.sys, gh(), [])).code, 0);
    assert.equal(d.asked.length, 2);
    assert.deepEqual(d.mime(), []);
    assert.equal(readFileSync(d.hypr, 'utf8'), 'local o = require("omarchy")\n');
  }

  // Without Hyprland there is no floating window question; without
  // xdg-mime there is no default viewer question.
  d = desktop(null);
  d.answer(['y']);
  assert.equal((await install(d.sys, gh(), [])).code, 0);
  assert.deepEqual(d.asked, [DEFAULT_QUESTION]);
  assert.ok(!existsSync(d.hypr), 'no Hyprland file is made');
  d = desktop();
  d.sys.env.PATH = path.join(d.sys.home, 'empty');
  d.answer(['y']);
  assert.equal((await install(d.sys, gh(), [])).code, 0);
  assert.deepEqual(d.asked, [FLOAT_QUESTION]);
  assert.deepEqual(d.mime(), []);

  // No terminal: no question and no change, and one line names --default.
  d = desktop();
  r = await install(d.sys, gh(), []);
  assert.equal(r.code, 0);
  assert.deepEqual(d.mime(), []);
  assert.equal(readFileSync(d.hypr, 'utf8'), 'local o = require("omarchy")\n');
  assert.ok(r.err.some((l) => l.includes('npx pandite --default')));

  // --no-default and --no-float ask nothing.
  d = desktop();
  d.answer(['y', 'y']);
  assert.equal((await install(d.sys, gh(), ['--no-default', '--no-float'])).code, 0);
  assert.deepEqual(d.asked, []);
  assert.deepEqual(d.mime(), []);

  // --default and --float act without a question, also without a terminal.
  d = desktop('-- mine');
  r = await install(d.sys, gh(), ['--default', '--float']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.deepEqual(d.mime().map((c) => c.args), [['default', DESKTOP_FILE, ...types]]);
  assert.equal(readFileSync(d.hypr, 'utf8'), `-- mine\n${rule}`, 'a missing final newline is added');
  // A second run with Pandite installed sets them again and adds no second rule.
  d.answer(['y', 'y']);
  r = await install(d.sys, gh(), ['--default', '--float']);
  assert.equal(r.code, 0);
  assert.deepEqual(d.asked, []);
  assert.equal(d.mime().length, 2);
  assert.equal(readFileSync(d.hypr, 'utf8'), `-- mine\n${rule}`);
  assert.ok(r.out.some((l) => l.includes('has a window rule for Pandite already')));
  // An update of an installed Pandite asks nothing.
  const newer = fakeGitHub([rel('0.3.0'), rel('0.2.0')], { latest: '0.3.0' });
  r = await install(d.sys, newer, ['update']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.deepEqual(d.asked, []);
  assert.equal(d.mime().length, 2);

  // --float without the Hyprland file, and a failing xdg-mime, do not fail the install.
  d = desktop(null);
  d.sys.state.xdgMimeExit = 3;
  r = await install(d.sys, gh(), ['--default', '--float']);
  assert.equal(r.code, 0);
  assert.ok(r.err.some((l) => l.includes('xdg-mime failed with exit code 3')), r.err.join('\n'));
  assert.ok(r.err.some((l) => l.includes('was not found; the floating window rule was not added')));
  assert.ok(existsSync(path.join(d.sys.home, '.local', 'bin', 'pandite')));

  // The dry run names the questions and changes nothing.
  d = desktop();
  d.answer(['y', 'y']);
  r = await install(d.sys, gh(), ['--dry-run']);
  assert.ok(r.out.includes('Would ask whether Pandite becomes the default image viewer'));
  assert.ok(r.out.includes('Would ask whether Pandite opens as a floating window in Hyprland'));
  assert.deepEqual(d.asked, []);
  r = await install(d.sys, gh(), ['--dry-run', '--default', '--float']);
  assert.ok(r.out.includes(`Would run ${XDG_MIME} default ${DESKTOP_FILE} with the MIME types of the desktop entry`));
  assert.ok(r.out.includes(`Would add the floating window rule for Pandite to ${d.hypr}`));
  assert.deepEqual(d.mime(), []);
  assert.equal(readFileSync(d.hypr, 'utf8'), 'local o = require("omarchy")\n');

  // The options are for Linux, and each pair excludes itself.
  for (const p of ['win32', 'darwin']) {
    r = await install(fakeSystem(p), gh(), ['--default']);
    assert.equal(r.code, 2);
    assert.ok(r.err.join('\n').includes('work on Linux only'));
  }
  assert.throws(() => parseArgs(['--default', '--no-default']), /choose one of --default and --no-default/);
  assert.throws(() => parseArgs(['--float', '--no-float']), /choose one of --float and --no-float/);
  assert.throws(() => parseArgs(['uninstall', '--default']), /uninstall takes no --default/);
});

test('--dry-run prints the plan and downloads, installs and changes nothing', async () => {
  for (const p of PLATFORMS) {
    const gh = fakeGitHub([rel('0.2.0')], { latest: '0.2.0' });
    const sys = fakeSystem(p.platform, p.arch);
    const r = await install(sys, gh, ['--dry-run']);
    assert.equal(r.code, 0, r.err.join('\n'));
    assert.deepEqual(gh.downloads(), [], 'nothing downloaded');
    assert.deepEqual(gh.apiCalls(), [`${API}/releases/latest`]);
    assert.ok(gh.calls.every((c) => c.url.startsWith('https://api.github.com/')), 'no release file requested');
    assert.deepEqual(programs(sys), [], 'no program started');
    assert.ok(!existsSync(sys.data), 'no data folder');
    noTempLeft(sys);
    const asset = chooseAsset({ ...p, glibc: '2.39', version: '0.2.0', available: [...releaseFiles('0.2.0').keys()] }).name;
    assert.equal(r.out[0], `Pandite 0.2.0 (latest release) for ${{ win32: 'Windows', darwin: 'macOS', linux: 'Linux' }[p.platform]} ${p.arch}`);
    assert.ok(r.out.includes(`Would download https://github.com/xls/pandite/releases/download/v0.2.0/${asset} (0.0 MB)`), r.out.join('\n'));
    assert.equal(r.out.at(-1), 'Dry run: nothing was downloaded, installed or changed.');
    if (p.platform === 'win32') {
      const line = r.out.find((l) => l.startsWith('Would run: '));
      assert.ok(line.endsWith(` ${SETUP_ARGS.join(' ')} /LOG=${path.join(sys.data, 'setup.log')}`), line);
    }
  }
  // update and uninstall too.
  const gh = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
  const sys = fakeSystem('win32', 'x64');
  assert.equal((await install(sys, gh, ['-v', '0.1.0'])).code, 0);
  const downloads = gh.downloads().length;
  let r = await install(sys, gh, ['update', '--dry-run']);
  assert.equal(r.code, 0);
  assert.equal(gh.downloads().length, downloads);
  assert.equal(sys.state.setupFiles.length, 1);
  r = await install(sys, gh, ['uninstall', '--dry-run']);
  assert.equal(r.code, 0);
  assert.deepEqual(r.out, [`Would run: ${sys.uninstaller} ${UNINSTALL_ARGS.join(' ')}`, `Would delete ${sys.data}`, 'Dry run: nothing was changed.']);
  assert.ok(sys.registry.has('HKCU') && existsSync(path.join(sys.data, MARKER)), 'still installed');
  assert.ok(!programs(sys).some((c) => c.command === sys.uninstaller));
});

test('the Windows setup program runs silently for the current user', async () => {
  const gh = fakeGitHub([rel('0.2.0')], { latest: '0.2.0' });
  const sys = fakeSystem('win32', 'x64');
  const r = await install(sys, gh, []);
  assert.equal(r.code, 0, r.err.join('\n'));
  const started = programs(sys);
  assert.equal(started.length, 1);
  assert.equal(path.basename(started[0].command), 'pandite-0.2.0-x86_64-pc-windows-msvc-setup.exe');
  assert.deepEqual(started[0].args, ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/CURRENTUSER', `/LOG=${path.join(sys.data, 'setup.log')}`]);
  assert.deepEqual(sys.state.setupFiles, [setupExe('0.2.0', 'x64')], 'the checked file runs');
  assert.deepEqual(r.out, [`Pandite 0.2.0 installed in ${sys.installDir}. Start it from the Start menu.`]);
  assert.equal(marker(sys).location, sys.installDir);
  // reg.exe reads the uninstall entry: HKCU first, then HKLM.
  const reg = sys.calls.filter((c) => c.command === regExe(sys.env)).map((c) => c.args[1]);
  assert.deepEqual(reg.slice(0, 2), [`HKCU\\${UNINSTALL_KEY}`, `HKLM\\${UNINSTALL_KEY}`]);
  assert.equal(UNINSTALL_KEY, 'Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{E5E712AA-D4BF-4DC9-BB19-9841DC38CD1E}_is1');
  noTempLeft(sys);
  // A failing setup program: its exit code and log, no marker.
  const fail = fakeSystem('win32', 'arm64');
  fail.state.setupExit = 5;
  const f = await install(fail, fakeGitHub([rel('0.2.0')], { latest: '0.2.0' }), []);
  assert.equal(f.code, 1);
  assert.equal(f.err.at(-1), `pandite-installer: the setup program failed with exit code 5. Its log is ${path.join(fail.data, 'setup.log')}`);
  assert.ok(!existsSync(path.join(fail.data, MARKER)));
  noTempLeft(fail);
});

test('the macOS app goes to ~/Applications, or to /Applications with --system', async () => {
  const gh = fakeGitHub([rel('0.2.0'), rel('0.1.0')], { latest: '0.2.0' });
  const sys = fakeSystem('darwin', 'arm64');
  assert.equal((await install(sys, gh, ['-v', '0.1.0'])).code, 0);
  const apps = path.join(sys.home, 'Applications');
  const app = path.join(apps, 'Pandite.app');
  const exe = path.join(app, 'Contents', 'MacOS', 'pandite');
  assert.equal(readFileSync(exe, 'utf8'), 'pandite 0.1.0 universal');
  writeFileSync(path.join(app, 'Contents', 'stale.txt'), 'from 0.1.0');
  const r = await install(sys, gh, ['update']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.equal(readFileSync(exe, 'utf8'), 'pandite 0.2.0 universal');
  assert.equal(readFileSync(path.join(app, 'Contents', 'Info.plist'), 'utf8'), '<plist>0.2.0</plist>');
  assert.ok(!existsSync(path.join(app, 'Contents', 'stale.txt')), 'the older copy is replaced, not merged');
  assert.deepEqual(readdirSync(apps), ['Pandite.app'], 'no staging folder and no __MACOSX');
  assert.deepEqual(r.out, [`Pandite 0.2.0 installed in ${app}. Start it from Launchpad or with: open "${app}"`]);
  assert.deepEqual(programs(sys), [], 'the zip unpacks without other programs');
  if (process.platform !== 'win32') assert.equal(readdirSync(path.dirname(exe)).length, 1);
  noTempLeft(sys);

  // --system, and update keeps that folder.
  const sys2 = fakeSystem('darwin', 'x64');
  assert.equal((await install(sys2, gh, ['-v', '0.1.0', '--system'])).code, 0);
  const sysApp = path.join(sys2.systemApplications, 'Pandite.app');
  assert.ok(existsSync(path.join(sysApp, 'Contents', 'MacOS', 'pandite')));
  assert.ok(!existsSync(path.join(sys2.home, 'Applications')));
  assert.equal(marker(sys2).system, true);
  assert.equal((await install(sys2, gh, ['update'])).code, 0);
  assert.equal(readFileSync(path.join(sysApp, 'Contents', 'MacOS', 'pandite'), 'utf8'), 'pandite 0.2.0 universal');
  assert.ok(!existsSync(path.join(sys2.home, 'Applications')));

  // A release without the zip: the disk image, through hdiutil.
  const dmg = fakeGitHub([rel('0.2.0', { omit: ['pandite-0.2.0-universal-apple-darwin.zip'] })], { latest: '0.2.0' });
  const sys3 = fakeSystem('darwin', 'arm64');
  const d = await install(sys3, dmg, []);
  assert.equal(d.code, 0, d.err.join('\n'));
  assert.deepEqual(dmg.downloads(), ['SHA256SUMS.txt', 'pandite-0.2.0-universal-apple-darwin.dmg']);
  const [attach, detach] = programs(sys3);
  const mount = attach.args[5];
  assert.deepEqual(attach.args, ['attach', '-nobrowse', '-readonly', '-noautoopen', '-mountpoint', mount, path.join(path.dirname(mount), 'pandite-0.2.0-universal-apple-darwin.dmg')]);
  assert.deepEqual(detach.args, ['detach', mount, '-force']);
  assert.equal(attach.command, HDIUTIL);
  assert.equal(readFileSync(path.join(sys3.home, 'Applications', 'Pandite.app', 'Contents', 'MacOS', 'pandite'), 'utf8'), 'pandite 0.2.0 from dmg');
  noTempLeft(sys3);
});

test('uninstall on each system', async () => {
  // Windows: the registered uninstaller, silent, then wait for its entry to go.
  const gh = fakeGitHub([rel('0.2.0')], { latest: '0.2.0' });
  const win = fakeSystem('win32', 'x64');
  assert.equal((await install(win, gh, [])).code, 0);
  win.state.entryLingers = 2;
  let r = await install(win, gh, ['uninstall']);
  assert.equal(r.code, 0, r.err.join('\n'));
  const ran = programs(win).filter((c) => c.command === win.uninstaller);
  assert.deepEqual(ran.map((c) => c.args), [['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART']]);
  assert.equal(win.state.sleeps, 2, 'waits while the entry stays');
  assert.deepEqual(r.out, [`Pandite removed from ${win.installDir}.`]);
  assert.ok(!existsSync(win.data), 'marker and log removed');
  r = await install(win, gh, ['--uninstall']);
  assert.equal(r.code, 0);
  assert.deepEqual(r.out, ['Pandite is not installed for this user: the setup program registered no uninstaller. Nothing was removed.']);
  // A failing uninstaller keeps the marker.
  const win2 = fakeSystem('win32', 'arm64');
  assert.equal((await install(win2, gh, [])).code, 0);
  win2.state.uninstallerExit = 3;
  r = await install(win2, gh, ['uninstall']);
  assert.equal(r.code, 1);
  assert.deepEqual(r.err, ['pandite-installer: running the uninstaller', 'pandite-installer: the uninstaller failed with exit code 3']);
  assert.ok(existsSync(path.join(win2.data, MARKER)));

  // Linux: the kept install.sh --uninstall with the same PREFIX.
  const lin = fakeSystem('linux');
  assert.equal((await install(lin, gh, [])).code, 0);
  const bin = path.join(lin.home, '.local', 'bin', 'pandite');
  assert.ok(existsSync(bin));
  r = await install(lin, gh, ['uninstall']);
  assert.equal(r.code, 0, r.err.join('\n'));
  const sh = programs(lin).at(-1);
  assert.deepEqual([sh.command, sh.args], [SH, [path.join(lin.data, 'linux', 'install.sh'), '--uninstall']]);
  assert.equal(sh.env.PREFIX, path.join(lin.home, '.local'));
  assert.ok(!existsSync(bin));
  assert.ok(!existsSync(lin.data));
  assert.deepEqual(r.out, [`Pandite removed from ${path.join(lin.home, '.local')}.`]);
  r = await install(lin, gh, ['uninstall']);
  assert.equal(r.code, 0);
  assert.match(r.out[0], /^Pandite is not installed by this installer: .* holds no install\.sh\. Nothing was removed\.$/);

  // macOS: delete the app.
  const mac = fakeSystem('darwin', 'arm64');
  assert.equal((await install(mac, gh, [])).code, 0);
  const app = path.join(mac.home, 'Applications', 'Pandite.app');
  r = await install(mac, gh, ['uninstall']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.ok(!existsSync(app));
  assert.ok(!existsSync(mac.data));
  assert.deepEqual(r.out, [`Pandite removed from ${app}.`]);
  assert.deepEqual(programs(mac), []);
  r = await install(mac, gh, ['uninstall']);
  assert.deepEqual(r.out, [`Pandite is not installed: ${app} does not exist. Nothing was removed.`]);
  assert.equal(gh.downloads().length, 4 * 2, 'four installs download two files each; uninstall downloads nothing');
});

test('--list prints the release versions, newest first', async () => {
  const gh = fakeGitHub([
    rel('0.9.0'),
    { version: '0.10.1-rc.1', prerelease: true, files: new Map() },
    rel('0.10.0'),
    { tag: 'nightly', files: new Map() },
    { version: '0.11.0', draft: true, files: new Map() },
  ]);
  const sys = fakeSystem('freebsd', 'x64');
  const r = await install(sys, gh, ['--list']);
  assert.equal(r.code, 0, r.err.join('\n'));
  assert.deepEqual(r.out, ['0.10.1-rc.1 (pre-release)', '0.10.0', '0.9.0']);
  assert.deepEqual(gh.apiCalls(), [`${API}/releases?per_page=100`]);
  const none = await install(fakeSystem('linux'), fakeGitHub([]), ['--list']);
  assert.equal(none.code, 1);
  assert.deepEqual(none.err, ['pandite-installer: no Pandite release found at https://github.com/xls/pandite/releases']);
});

test('no release, rate limit and network errors give one clear line', async () => {
  const cases = [
    [fakeGitHub([]).fetch, 'no Pandite release found at https://github.com/xls/pandite/releases'],
    [fakeGitHub([], { rateLimit: true }).fetch, 'the GitHub API rate limit for this network is used up (60 requests per hour without sign-in). Try again in 25 minutes.'],
    [fakeGitHub([], { apiStatus: 500 }).fetch, `GitHub answered HTTP 500 for ${API}/releases/latest`],
    [
      async () => {
        throw new TypeError('fetch failed', { cause: Object.assign(new Error('getaddrinfo ENOTFOUND api.github.com'), { code: 'ENOTFOUND' }) });
      },
      'cannot reach api.github.com: getaddrinfo ENOTFOUND api.github.com. Check the network connection.',
    ],
  ];
  for (const [fetch, message] of cases) {
    for (const argv of [[], ['update'], ['--dry-run'], ['--list']]) {
      const sys = fakeSystem('linux');
      const r = await install(sys, { fetch }, argv);
      if (argv[0] === '--list' && message.startsWith('GitHub answered')) {
        assert.deepEqual(r.err, [`pandite-installer: GitHub answered HTTP 500 for ${API}/releases?per_page=100`]);
      } else {
        assert.deepEqual(r.err, [`pandite-installer: ${message}`], argv.join(' '));
      }
      assert.equal(r.code, 1);
      assert.deepEqual(r.out, []);
      assert.deepEqual(programs(sys), []);
    }
  }
});

test('the command line', async () => {
  const sys = fakeSystem('linux');
  const gh = fakeGitHub([]);
  let r = await install(sys, gh, ['--help']);
  assert.equal(r.code, 0);
  assert.ok(r.out[0].startsWith(`Pandite installer ${INSTALLER_VERSION}\n`));
  for (const word of ['install', 'update', 'uninstall', '--uninstall', '--list', '--dry-run', '--force', '--system', '--installer-version', '-v, --version']) {
    assert.ok(r.out[0].includes(word), `--help names ${word}`);
  }
  r = await install(sys, gh, ['--installer-version']);
  assert.deepEqual(r.out, [JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version]);
  for (const [argv, says] of [
    [['--frobnicate'], 'unknown option --frobnicate'],
    [['photo.jpg'], 'unknown command "photo.jpg". This is the Pandite installer; it opens no images'],
    [['--system'], '--system works on macOS only'],
    [['update', '-v', '1.0'], 'update takes no version. To install a named version, run: npx pandite install -v 1.0.0'],
    [['uninstall', '-v', '1.0'], 'uninstall takes no version'],
    [['install', 'update'], 'choose one command, not install and update'],
    [['-v'], '-v needs a Pandite version, such as -v 0.1.0 (for the version of the installer: --installer-version)'],
    [['--version', '--force'], '--version needs a Pandite version'],
    [['--force=yes'], '--force takes no value'],
  ]) {
    r = await install(sys, gh, argv);
    assert.equal(r.code, 2, argv.join(' '));
    assert.equal(r.err.length, 1);
    assert.ok(r.err[0].startsWith(`pandite-installer: ${says}`), r.err[0]);
    assert.ok(r.err[0].endsWith('Run "npx pandite --help" for the options.'));
  }
  assert.equal(gh.calls.length, 0);
  assert.deepEqual(parseArgs([]), { command: null, version: null, list: false, dryRun: false, force: false, system: false, makeDefault: false, noDefault: false, float: false, noFloat: false, installerVersion: false, help: false });
  assert.equal(parseArgs(['--uninstall']).command, 'uninstall');
  assert.equal(parseArgs(['uninstall', '--uninstall']).command, 'uninstall');
  assert.equal(parseArgs(['-v', 'v0.1.0', '--force', '--system', '--dry-run']).version, '0.1.0');
  assert.throws(() => parseArgs(['-v', 'abc']), (e) => e instanceof InstallerError && e.exitCode === 2);
});

test('archive entries outside their folder and links are refused', () => {
  const bad = [
    makeZip([{ name: '../evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: '/abs/evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: 'C:/evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: 'a\\..\\..\\evil.txt', data: Buffer.from('x') }]),
    makeZip([{ name: 'link', data: Buffer.from('/etc/passwd'), mode: 0o120777 }]),
  ];
  for (const zip of bad) assert.throws(() => unzip(zip, tempDir()), InstallerError);
  assert.throws(() => untar(makeTarGz([{ name: '../evil', data: Buffer.from('x') }]), tempDir()), InstallerError);
  assert.throws(() => untar(makeTarGz([{ name: 'link', type: '2' }]), tempDir()), InstallerError);
  assert.throws(() => untar(makeTarGz([{ name: 'hard', type: '1' }]), tempDir()), InstallerError);
  assert.throws(() => unzip(Buffer.from('not a zip at all, not even close to one'), tempDir()), /not a zip file/);
  const dest = tempDir();
  const long = `top/${'d'.repeat(120)}/file.txt`;
  assert.deepEqual(untar(makeTarGz([{ name: long, data: Buffer.from('long') }]), dest), [long]);
  assert.equal(readFileSync(path.join(dest, ...long.split('/')), 'utf8'), 'long');
});

test('SHA256SUMS.txt and reg export files parse', () => {
  const sums = parseSums(`${'A'.repeat(64)}  a.zip\n${'b'.repeat(64)} *b file.exe\r\nnot a line\n\n`);
  assert.deepEqual([...sums], [['a.zip', 'a'.repeat(64)], ['b file.exe', 'b'.repeat(64)]]);
  // A user folder with a character outside ASCII survives (reg export writes UTF-16).
  const location = 'C:\\Users\\Jos\u00e9 \\"Q\\"\\AppData\\Local\\Programs\\Pandite\\';
  const values = parseRegFile(regFile(`HKCU\\${UNINSTALL_KEY}`, { InstallLocation: location, DisplayVersion: '0.1.0' }));
  assert.deepEqual(values, { InstallLocation: location, DisplayVersion: '0.1.0' });
});

let failed = 0;
try {
  for (const [name, fn] of tests) {
    try {
      await fn();
      console.log(`ok - ${name}`);
    } catch (error) {
      failed += 1;
      console.error(`STEP FAILED: ${name}\n${error?.stack ?? error}`);
    }
  }
} finally {
  rmSync(TEMP, { recursive: true, force: true, maxRetries: 3 });
}
if (failed) {
  console.error(`${failed} of ${tests.length} tests failed`);
  process.exit(1);
}
console.log('NPM INSTALLER VERIFIED');
