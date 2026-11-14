#!/usr/bin/env node
// Build an MSIX package (or an MSIX bundle) of pandite with the Windows SDK.
//
// Pack one package:
//   node packaging/windows/build-msix.mjs [--build] [--target <triple>] [--exe <pandite.exe>]
//        [--arch x64|arm64] [--version <x.y.z>] [--out <dir>] [--name <file.msix>]
//        [--identity-name <name>] [--publisher <CN=...>] [--publisher-display-name <text>]
//        [--require-identity] [--require-static-crt] [--keep-stage]
//
// --require-identity fails when a Partner Center value is missing.
// --require-static-crt fails when the binary needs the Visual C++
// Redistributable (build with RUSTFLAGS="-C target-feature=+crt-static").
// --require-identity implies --require-static-crt.
//
// Bundle packages (one per architecture) into an .msixbundle:
//   node packaging/windows/build-msix.mjs --bundle <a.msix> <b.msix> [--version <x.y.z>]
//        [--out <dir>] [--name <file.msixbundle>]
//
// Gate mode: release build, unsigned x64 package in dist/, container check:
//   node packaging/windows/build-msix.mjs --check
//
// The three Partner Center values also come from the environment:
//   PANDITE_MSIX_IDENTITY_NAME, PANDITE_MSIX_PUBLISHER,
//   PANDITE_MSIX_PUBLISHER_DISPLAY_NAME. Arguments win over variables.
// The package is unsigned. The Microsoft Store signs packages it accepts.
// For sideloading, sign it with packaging/windows/sign-msix.mjs.

import { spawnSync } from 'node:child_process';
import {
  copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync,
  writeFileSync,
} from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { basename, delimiter, dirname, join, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { inflateRawSync } from 'node:zlib';
import { findSdkTool, missingToolMessage } from './sdk-tools.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..', '..');
const MANIFEST = join(HERE, 'AppxManifest.xml');
const ASSETS = join(HERE, 'assets');

const ARCH_TRIPLE = { x64: 'x86_64-pc-windows-msvc', arm64: 'aarch64-pc-windows-msvc' };
const TRIPLE_ARCH = Object.fromEntries(Object.entries(ARCH_TRIPLE).map(([a, t]) => [t, a]));
const PLACEHOLDER = /REPLACE-ME/;

class StepFailed extends Error {}

/** Stop the build. Thrown, so `finally` blocks remove temporary folders. */
function fail(message) {
  throw new StepFailed(message);
}

// ---------------------------------------------------------------- arguments

function parseArgs(argv) {
  const opts = { bundle: null };
  const valued = new Set([
    '--exe', '--arch', '--version', '--out', '--name', '--target', '--identity-name', '--publisher',
    '--publisher-display-name',
  ]);
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--bundle') {
      opts.bundle = [];
      while (i + 1 < argv.length && !argv[i + 1].startsWith('--')) opts.bundle.push(argv[(i += 1)]);
    } else if (valued.has(a)) {
      if (i + 1 >= argv.length) fail(`${a} needs a value`);
      opts[a.slice(2)] = argv[(i += 1)];
    } else if (['--check', '--build', '--require-identity', '--require-static-crt', '--keep-stage'].includes(a)) {
      opts[a.slice(2)] = true;
    } else if (a === '--help' || a === '-h') {
      const source = readFileSync(fileURLToPath(import.meta.url), 'utf8').replace(/^#!.*\n/, '');
      console.log(source.split('\n\n')[0].replace(/^\/\/ ?/gm, ''));
      process.exit(0);
    } else {
      fail(`unknown argument: ${a}`);
    }
  }
  return opts;
}

// ------------------------------------------------------------------ helpers

function workspaceVersion() {
  const toml = readFileSync(join(ROOT, 'Cargo.toml'), 'utf8');
  const section = toml.split(/^\[workspace\.package\]\s*$/m)[1] ?? '';
  const m = section.split(/^\[/m)[0].match(/^version\s*=\s*"([^"]+)"/m);
  if (!m) fail('cannot read [workspace.package] version from Cargo.toml');
  return m[1];
}

/** "1.2.3" or "1.2.3-rc.1" -> "1.2.3.0". The Store reserves the fourth part. */
export function msixVersion(version) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(version.trim());
  if (!m) fail(`version "${version}" is not x.y.z`);
  const parts = m.slice(1, 4).map(Number);
  if (parts.some((p) => p > 65535)) fail(`version "${version}" has a part above 65535`);
  return `${parts.join('.')}.0`;
}

function xmlEscape(text) {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function run(label, exe, args, options = {}) {
  const res = spawnSync(exe, args, { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, ...options });
  if (res.error || res.status !== 0) {
    const out = `${res.stdout ?? ''}\n${res.stderr ?? ''}`.split(/\r?\n/).filter(Boolean).slice(-40).join('\n');
    fail(`${label} (exit ${res.status})\n${res.error ? res.error.message : ''}\n${out}`);
  }
  return res;
}

function sdkTool(tool) {
  let path;
  try {
    path = findSdkTool(tool);
  } catch (err) {
    fail(err.message);
  }
  if (!path) fail(missingToolMessage(tool));
  return path;
}

// ------------------------------------------------------------ PE inspection

/** Names (lowercase) of the DLLs a PE executable imports directly. */
export function peImports(buf) {
  if (buf.length < 0x40 || buf.toString('latin1', 0, 2) !== 'MZ') throw new Error('not a PE file');
  const pe = buf.readUInt32LE(0x3c);
  if (buf.toString('latin1', pe, pe + 4) !== 'PE\0\0') throw new Error('no PE signature');
  const sections = buf.readUInt16LE(pe + 6);
  const optSize = buf.readUInt16LE(pe + 20);
  const opt = pe + 24;
  const magic = buf.readUInt16LE(opt);
  const dirs = opt + (magic === 0x20b ? 112 : 96);
  const dirCount = buf.readUInt32LE(dirs - 4);
  if (dirCount < 2) return [];
  const importRva = buf.readUInt32LE(dirs + 8);
  if (importRva === 0) return [];
  const table = opt + optSize;
  const toOffset = (rva) => {
    for (let s = 0; s < sections; s += 1) {
      const h = table + s * 40;
      const va = buf.readUInt32LE(h + 12);
      const size = Math.max(buf.readUInt32LE(h + 8), buf.readUInt32LE(h + 16));
      if (rva >= va && rva < va + size) return rva - va + buf.readUInt32LE(h + 20);
    }
    throw new Error(`RVA 0x${rva.toString(16)} is outside every section`);
  };
  const names = [];
  for (let d = toOffset(importRva); ; d += 20) {
    const nameRva = buf.readUInt32LE(d + 12);
    if (nameRva === 0) break;
    const at = toOffset(nameRva);
    names.push(buf.toString('latin1', at, buf.indexOf(0, at)).toLowerCase());
  }
  return names;
}

/** Fail or warn when the binary needs the Visual C++ Redistributable. */
function checkCrt(exe, required) {
  let imports;
  try {
    imports = peImports(readFileSync(exe));
  } catch (err) {
    fail(`${exe} is not a Windows executable: ${err.message}`);
  }
  const vc = imports.filter((n) => /^(vcruntime|msvcp)\d+(_\d+)?\.dll$/.test(n));
  if (vc.length === 0) return;
  const text = `${exe} imports ${vc.join(', ')} from the Visual C++ Redistributable, which a clean`
    + ' Windows lacks. Build with RUSTFLAGS="-C target-feature=+crt-static" (the release workflow does).';
  if (required) fail(text);
  console.log(`note: ${text}`);
}

// ----------------------------------------------------------- zip inspection

/** Central directory entries of a ZIP or ZIP64 file. */
export function zipEntries(buf) {
  const minEocd = 22;
  let eocd = -1;
  for (let i = buf.length - minEocd; i >= Math.max(0, buf.length - minEocd - 65535); i -= 1) {
    if (buf.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error('no end of central directory record');
  let count = buf.readUInt16LE(eocd + 10);
  let cdOffset = buf.readUInt32LE(eocd + 16);
  if (count === 0xffff || cdOffset === 0xffffffff) {
    const loc = eocd - 20;
    if (loc < 0 || buf.readUInt32LE(loc) !== 0x07064b50) throw new Error('no ZIP64 locator');
    const rec = Number(buf.readBigUInt64LE(loc + 8));
    if (buf.readUInt32LE(rec) !== 0x06064b50) throw new Error('no ZIP64 end of central directory');
    count = Number(buf.readBigUInt64LE(rec + 32));
    cdOffset = Number(buf.readBigUInt64LE(rec + 48));
  }
  const entries = [];
  let p = cdOffset;
  for (let n = 0; n < count; n += 1) {
    if (buf.readUInt32LE(p) !== 0x02014b50) throw new Error(`bad central directory header ${n}`);
    const method = buf.readUInt16LE(p + 10);
    let compSize = buf.readUInt32LE(p + 20);
    let size = buf.readUInt32LE(p + 24);
    const nameLen = buf.readUInt16LE(p + 28);
    const extraLen = buf.readUInt16LE(p + 30);
    const commentLen = buf.readUInt16LE(p + 32);
    let offset = buf.readUInt32LE(p + 42);
    const name = buf.toString('utf8', p + 46, p + 46 + nameLen);
    let e = p + 46 + nameLen;
    const extraEnd = e + extraLen;
    while (e + 4 <= extraEnd) {
      const id = buf.readUInt16LE(e);
      const len = buf.readUInt16LE(e + 2);
      if (id === 0x0001) {
        let q = e + 4;
        if (size === 0xffffffff) { size = Number(buf.readBigUInt64LE(q)); q += 8; }
        if (compSize === 0xffffffff) { compSize = Number(buf.readBigUInt64LE(q)); q += 8; }
        if (offset === 0xffffffff) { offset = Number(buf.readBigUInt64LE(q)); q += 8; }
      }
      e += 4 + len;
    }
    entries.push({ name, method, compSize, size, offset });
    p = extraEnd + commentLen;
  }
  return entries;
}

/** The uncompressed bytes of one entry. */
export function zipRead(buf, entry) {
  if (buf.readUInt32LE(entry.offset) !== 0x04034b50) throw new Error(`bad local header for ${entry.name}`);
  const start = entry.offset + 30 + buf.readUInt16LE(entry.offset + 26) + buf.readUInt16LE(entry.offset + 28);
  const data = buf.subarray(start, start + entry.compSize);
  if (entry.method === 0) return data;
  if (entry.method === 8) return inflateRawSync(data);
  throw new Error(`unsupported compression method ${entry.method} for ${entry.name}`);
}

function verifyPackage(file, { version, arch }) {
  if (!existsSync(file)) fail(`package not found: ${file}`);
  const buf = readFileSync(file);
  if (buf.length < 22 || buf.readUInt32LE(0) !== 0x04034b50) fail(`${file} is not a ZIP container`);
  let entries;
  try {
    entries = zipEntries(buf);
  } catch (err) {
    fail(`${file} is not a valid ZIP container: ${err.message}`);
  }
  const names = new Set(entries.map((e) => e.name));
  const required = [
    'AppxManifest.xml', 'AppxBlockMap.xml', '[Content_Types].xml', 'pandite.exe', 'resources.pri',
    'assets/StoreLogo.png', 'assets/Square44x44Logo.png', 'assets/Square150x150Logo.png',
  ];
  const missing = required.filter((n) => !names.has(n));
  if (missing.length) fail(`${file} lacks: ${missing.join(', ')}`);
  const manifest = zipRead(buf, entries.find((e) => e.name === 'AppxManifest.xml')).toString('utf8');
  if (!manifest.includes(`Version="${version}"`)) fail(`packaged manifest does not carry Version="${version}"`);
  if (!manifest.includes(`ProcessorArchitecture="${arch}"`)) {
    fail(`packaged manifest does not carry ProcessorArchitecture="${arch}"`);
  }
  if (/internetClient|privateNetworkClientServer/.test(manifest)) fail('packaged manifest declares a network capability');
  return entries.length;
}

function verifyBundle(file, version) {
  if (!existsSync(file)) fail(`bundle not found: ${file}`);
  const buf = readFileSync(file);
  let entries;
  try {
    entries = zipEntries(buf);
  } catch (err) {
    fail(`${file} is not a valid ZIP container: ${err.message}`);
  }
  const bm = entries.find((e) => e.name === 'AppxMetadata/AppxBundleManifest.xml');
  if (!bm) fail(`${file} lacks AppxMetadata/AppxBundleManifest.xml`);
  const text = zipRead(buf, bm).toString('utf8');
  if (!text.includes(`Version="${version}"`)) fail(`bundle manifest does not carry Version="${version}"`);
  return entries.filter((e) => e.name.endsWith('.msix')).length;
}

// ------------------------------------------------------------------ identity

function identity(opts) {
  const pick = (arg, env) => (opts[arg] || process.env[env] || '').trim();
  return {
    name: pick('identity-name', 'PANDITE_MSIX_IDENTITY_NAME'),
    publisher: pick('publisher', 'PANDITE_MSIX_PUBLISHER'),
    publisherDisplayName: pick('publisher-display-name', 'PANDITE_MSIX_PUBLISHER_DISPLAY_NAME'),
  };
}

/** The checked-in manifest with version, architecture and identity filled in. */
export function renderManifest(template, { version, arch, name, publisher, publisherDisplayName }) {
  const identityRe = /<Identity\b[\s\S]*?\/>/;
  const m = identityRe.exec(template);
  if (!m) fail('AppxManifest.xml has no <Identity .../> element');
  let id = m[0];
  const setAttr = (el, attr, value) => {
    const re = new RegExp(`(\\b${attr}=")[^"]*(")`);
    if (!re.test(el)) fail(`<Identity> has no ${attr} attribute`);
    return el.replace(re, (_, a, b) => `${a}${xmlEscape(value)}${b}`);
  };
  id = setAttr(id, 'Version', version);
  id = setAttr(id, 'ProcessorArchitecture', arch);
  if (name) id = setAttr(id, 'Name', name);
  if (publisher) id = setAttr(id, 'Publisher', publisher);
  let out = template.replace(identityRe, () => id);
  if (publisherDisplayName) {
    const pdn = /<PublisherDisplayName>[\s\S]*?<\/PublisherDisplayName>/;
    if (!pdn.test(out)) fail('AppxManifest.xml has no <PublisherDisplayName>');
    out = out.replace(pdn, () => `<PublisherDisplayName>${xmlEscape(publisherDisplayName)}</PublisherDisplayName>`);
  }
  return out;
}

function checkIdentity(id, required) {
  if (id.name && !/^[-.A-Za-z0-9]{3,50}$/.test(id.name)) {
    fail(`identity name "${id.name}" must be 3 to 50 characters from A-Z, a-z, 0-9, "." and "-"`);
  }
  if (id.publisher && !/^CN=/.test(id.publisher)) fail(`publisher "${id.publisher}" must start with CN=`);
  if (id.publisherDisplayName && id.publisherDisplayName.length > 256) fail('publisher display name is longer than 256 characters');
  const placeholders = [];
  if (!id.name) placeholders.push('identity name');
  if (!id.publisher) placeholders.push('publisher');
  if (!id.publisherDisplayName) placeholders.push('publisher display name');
  if (placeholders.length) {
    const text = `Partner Center values not set, placeholders kept: ${placeholders.join(', ')}`;
    if (required) fail(`${text}. See docs/store-submission.md.`);
    console.log(`note: ${text}. The Store rejects a package with placeholders.`);
  }
}

// --------------------------------------------------------------------- cargo

function cargoEnv() {
  const env = { ...process.env };
  env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
  delete env.Path;
  env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(ROOT, 'target');
  env.CARGO_TERM_COLOR = 'never';
  return env;
}

function releaseBuild(target) {
  const env = cargoEnv();
  const args = ['build', '--release', '-p', 'pandite-app'];
  if (target) args.push('--target', target);
  console.log(`cargo ${args.join(' ')} (target dir ${env.CARGO_TARGET_DIR})`);
  run('release build', 'cargo', args, { env, cwd: ROOT });
  const exe = join(env.CARGO_TARGET_DIR, ...(target ? [target] : []), 'release', 'pandite.exe');
  if (!existsSync(exe)) fail(`binary not found at ${exe}`);
  return exe;
}

// ---------------------------------------------------------------------- pack

function stageAndPack({ exe, arch, version, outDir, fileName, id, keepStage }) {
  const makeappx = sdkTool('makeappx.exe');
  const makepri = sdkTool('makepri.exe');
  if (!existsSync(exe)) fail(`binary not found: ${exe}`);
  if (!existsSync(ASSETS)) fail(`logo folder missing: ${ASSETS} (run node packaging/make-assets.mjs)`);

  const work = mkdtempSync(join(tmpdir(), 'pandite-msix-'));
  const stage = join(work, 'package');
  mkdirSync(join(stage, 'assets'), { recursive: true });
  try {
    copyFileSync(exe, join(stage, 'pandite.exe'));
    const pngs = readdirSync(ASSETS).filter((f) => f.toLowerCase().endsWith('.png'));
    if (pngs.length === 0) fail(`no PNG logos in ${ASSETS}`);
    for (const f of pngs) copyFileSync(join(ASSETS, f), join(stage, 'assets', f));
    copyFileSync(join(ROOT, 'LICENSE'), join(stage, 'LICENSE.txt'));
    const notices = join(ROOT, 'docs', 'THIRD-PARTY-NOTICES.md');
    if (existsSync(notices)) copyFileSync(notices, join(stage, 'THIRD-PARTY-NOTICES.md'));

    const manifest = renderManifest(readFileSync(MANIFEST, 'utf8'), {
      version, arch, name: id.name, publisher: id.publisher, publisherDisplayName: id.publisherDisplayName,
    });
    writeFileSync(join(stage, 'AppxManifest.xml'), manifest);

    // Resource index: lets Windows pick the scale-* and targetsize-* logos.
    const priConfig = join(work, 'priconfig.xml');
    run('makepri createconfig', makepri, ['createconfig', '/cf', priConfig, '/dq', 'en-US', '/pv', '10.0.0', '/o']);
    // Keep every resource in the main package (no resource packs).
    const config = readFileSync(priConfig, 'utf8').replace(/<packaging>[\s\S]*?<\/packaging>/, '');
    writeFileSync(priConfig, config);
    run('makepri new', makepri, [
      'new', '/pr', stage, '/cf', priConfig, '/mn', join(stage, 'AppxManifest.xml'),
      '/of', join(stage, 'resources.pri'), '/o',
    ]);

    mkdirSync(outDir, { recursive: true });
    const out = join(outDir, fileName);
    rmSync(out, { force: true });
    console.log(`makeappx: ${makeappx}`);
    run('makeappx pack', makeappx, ['pack', '/d', stage, '/p', out, '/o']);
    if (!existsSync(out) || statSync(out).size === 0) fail(`makeappx exited 0 but ${out} is missing or empty`);
    return out;
  } finally {
    if (keepStage) console.log(`stage kept: ${stage}`);
    else rmSync(work, { recursive: true, force: true });
  }
}

function bundle({ packages, version, outDir, fileName }) {
  const makeappx = sdkTool('makeappx.exe');
  if (packages.length === 0) fail('--bundle needs at least one .msix file');
  const work = mkdtempSync(join(tmpdir(), 'pandite-bundle-'));
  try {
    for (const p of packages) {
      if (!existsSync(p)) fail(`package not found: ${p}`);
      copyFileSync(p, join(work, basename(p)));
    }
    mkdirSync(outDir, { recursive: true });
    const out = join(outDir, fileName);
    rmSync(out, { force: true });
    run('makeappx bundle', makeappx, ['bundle', '/d', work, '/p', out, '/bv', version, '/o']);
    if (!existsSync(out)) fail(`makeappx exited 0 but ${out} is missing`);
    return out;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------- main

function main() {
  if (process.platform !== 'win32') fail('MSIX packages are built on Windows (makeappx.exe is part of the Windows SDK)');
  const opts = parseArgs(process.argv.slice(2));
  const semver = opts.version || workspaceVersion();
  const version = msixVersion(semver);
  const display = semver.replace(/^v/, '');
  const outDir = resolve(process.cwd(), opts.out || join(ROOT, 'dist'));

  if (opts.bundle) {
    const fileName = opts.name || `pandite-${display}-windows.msixbundle`;
    const out = bundle({ packages: opts.bundle.map((p) => resolve(p)), version, outDir, fileName });
    const count = verifyBundle(out, version);
    console.log(`${out}: bundle of ${count} packages, version ${version}`);
    console.log('MSIX BUNDLE BUILT');
    return;
  }

  if (opts.check) opts.build = true;
  // Without --arch or --target, cargo builds for the host, so package the host architecture.
  const hostArch = process.arch === 'arm64' ? 'arm64' : 'x64';
  let arch = opts.arch || (opts.target ? TRIPLE_ARCH[opts.target] : hostArch);
  if (!ARCH_TRIPLE[arch]) fail(`--arch must be x64 or arm64, not "${arch}"`);
  if (opts.target && TRIPLE_ARCH[opts.target] !== arch) fail(`--target ${opts.target} does not match --arch ${arch}`);
  const id = identity(opts);
  checkIdentity(id, Boolean(opts['require-identity']));

  let exe = opts.exe ? resolve(opts.exe) : null;
  if (opts.build) {
    if (exe) fail('use --exe or --build, not both');
    exe = releaseBuild(opts.target);
  }
  if (!exe) {
    const dir = process.env.PANDITE_TARGET_DIR || join(ROOT, 'target');
    exe = join(dir, ...(opts.target ? [opts.target] : []), 'release', 'pandite.exe');
  }
  if (!existsSync(exe)) fail(`binary not found: ${exe}`);
  // A Store package must run on a clean system: no separate C runtime.
  checkCrt(exe, Boolean(opts['require-static-crt'] || opts['require-identity']));
  // The file name says windows, not the full target: x86_64-windows.msix.
  const fileName = opts.name || `pandite-${display}-${ARCH_TRIPLE[arch].replace('-pc-windows-msvc', '-windows')}.msix`;
  const out = stageAndPack({ exe, arch, version, outDir, fileName, id, keepStage: opts['keep-stage'] });
  const entries = verifyPackage(out, { version, arch });
  console.log(`${out}: ${statSync(out).size} bytes, ${entries} entries, version ${version}, ${arch}, unsigned`);
  console.log('MSIX BUILT');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (err) {
    console.error(`STEP FAILED: ${err instanceof StepFailed ? err.message : err.stack}`);
    process.exit(1);
  }
}
