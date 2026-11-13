#!/usr/bin/env node
// Consistency check of the packaging definitions. Node standard library
// only, no cargo, runs on any system.
//
// Usage: node packaging/check.mjs
//
// Checks:
//  - the MSIX manifest associates exactly the extensions in
//    packaging/extensions.txt, declares no network capability, and every
//    logo it names exists with all scale-* (and targetsize-*) variants at
//    the right pixel sizes
//  - packaging/macos/Info.plist lists exactly the same extensions
//  - packaging/linux/pandite.desktop covers the same extensions through its
//    MIME types, minus the formats Linux cannot decode
//  - every packaging script the workflows call exists, and every action is
//    pinned to a major version
//  - packaging, workflow and documentation text files are pure ASCII
//    without tabs in YAML

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const errors = [];
const err = (msg) => errors.push(msg);
const read = (rel) => readFileSync(join(ROOT, rel), 'utf8');
const setEq = (a, b) => a.size === b.size && [...a].every((x) => b.has(x));
const diff = (a, b) => [...a].filter((x) => !b.has(x)).sort();

// HEIC and HEIF decode through the operating system codec (WIC, ImageIO).
// Linux has none, so the desktop entry leaves them out.
const LINUX_EXCLUDED = new Set(['heic', 'heif', 'hif']);

const MIME_EXT = {
  'image/jpeg': ['jpg', 'jpeg', 'jpe', 'jfif'],
  'image/pjpeg': ['jpg', 'jpeg', 'jpe', 'jfif'],
  'image/png': ['png'],
  'image/apng': ['apng'],
  'image/vnd.mozilla.apng': ['apng'],
  'image/gif': ['gif'],
  'image/webp': ['webp'],
  'image/bmp': ['bmp', 'dib'],
  'image/x-bmp': ['bmp', 'dib'],
  'image/x-ms-bmp': ['bmp', 'dib'],
  'image/tiff': ['tif', 'tiff'],
  'image/x-tga': ['tga'],
  'image/x-targa': ['tga'],
  'image/vnd.microsoft.icon': ['ico'],
  'image/x-icon': ['ico'],
  'image/x-win-bitmap': ['cur'],
  'image/qoi': ['qoi'],
  'image/x-qoi': ['qoi'],
  'image/avif': ['avif'],
  'image/heic': ['heic'],
  'image/heif': ['heif', 'hif'],
  'image/jxl': ['jxl'],
  'image/x-dds': ['dds'],
  'image/ktx2': ['ktx2'],
  'image/x-exr': ['exr'],
  'image/vnd.radiance': ['hdr'],
  'image/x-hdr': ['hdr'],
  'image/svg+xml': ['svg'],
  'image/vnd.adobe.photoshop': ['psd'],
  'image/x-psd': ['psd'],
  'image/x-adobe-dng': ['dng'],
  'image/x-canon-cr2': ['cr2'],
  'image/x-canon-cr3': ['cr3'],
  'image/x-nikon-nef': ['nef'],
  'image/x-sony-arw': ['arw'],
  'image/x-fuji-raf': ['raf'],
  'image/x-olympus-orf': ['orf'],
  'image/x-panasonic-rw2': ['rw2'],
  'image/x-pentax-pef': ['pef'],
  'image/x-samsung-srw': ['srw'],
};

// Manifest logo attribute -> size at scale 100.
const LOGO_SIZES = {
  Logo: [50, 50],
  Square44x44Logo: [44, 44],
  Square71x71Logo: [71, 71],
  Square150x150Logo: [150, 150],
  Wide310x150Logo: [310, 150],
  Square310x310Logo: [310, 310],
};
const SCALES = [100, 125, 150, 200, 400];
const TARGET_SIZES = [16, 24, 32, 48, 256];

function pngSize(path) {
  const b = readFileSync(path);
  if (b.length < 24 || b.toString('hex', 0, 8) !== '89504e470d0a1a0a') return null;
  return [b.readUInt32BE(16), b.readUInt32BE(20)];
}

function checkLogo(dir, rel, [w, h], targetSizes) {
  const base = rel.replace(/\\/g, '/').replace(/\.png$/i, '');
  const want = [[`${base}.png`, w, h]];
  for (const s of SCALES) want.push([`${base}.scale-${s}.png`, Math.round((w * s) / 100), Math.round((h * s) / 100)]);
  if (targetSizes) {
    for (const n of TARGET_SIZES) {
      for (const suffix of ['', '_altform-unplated', '_altform-lightunplated']) want.push([`${base}.targetsize-${n}${suffix}.png`, n, n]);
    }
  }
  for (const [file, ww, hh] of want) {
    const p = join(dir, file);
    if (!existsSync(p)) {
      err(`logo missing: ${relative(ROOT, p)}`);
      continue;
    }
    const size = pngSize(p);
    if (!size) err(`not a PNG: ${relative(ROOT, p)}`);
    else if (size[0] !== ww || size[1] !== hh) err(`${relative(ROOT, p)} is ${size[0]}x${size[1]}, want ${ww}x${hh}`);
  }
}

// ---------------------------------------------------------------- extensions

const extList = read('packaging/extensions.txt').split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
const exts = new Set(extList);
if (exts.size !== extList.length) err('packaging/extensions.txt has duplicates');
for (const e of extList) if (!/^[a-z0-9]+$/.test(e)) err(`extensions.txt: "${e}" is not a lowercase extension`);

// ------------------------------------------------------------------ manifest

const manifestRel = 'packaging/windows/AppxManifest.xml';
const manifest = read(manifestRel);
const body = manifest.replace(/<!--[\s\S]*?-->/g, '');
const fileTypes = [...body.matchAll(/<uap:FileType[^>]*>\s*\.([^<\s]+)\s*<\/uap:FileType>/g)].map((m) => m[1]);
const ftSet = new Set(fileTypes);
if (ftSet.size !== fileTypes.length) err('manifest lists an extension twice');
if (!setEq(ftSet, exts)) {
  err(`manifest file types differ from extensions.txt: missing [${diff(exts, ftSet)}] extra [${diff(ftSet, exts)}]`);
}
const ftaNames = [...body.matchAll(/FileTypeAssociation Name="([^"]+)"/g)].map((m) => m[1]);
if (new Set(ftaNames).size !== ftaNames.length) err('manifest repeats a FileTypeAssociation Name');
for (const n of ftaNames) if (!/^[-_.a-z0-9]{1,100}$/.test(n)) err(`FileTypeAssociation Name "${n}" is not [-_.a-z0-9]`);
if (/internetClient|privateNetworkClientServer|internetClientServer/.test(body)) err('manifest declares a network capability');
const caps = [...body.matchAll(/<(?:\w+:)?Capability\b[^>]*Name="([^"]+)"/g)].map((m) => m[1]);
if (caps.join(',') !== 'runFullTrust') err(`manifest capabilities must be exactly runFullTrust, found [${caps}]`);
const manifestDir = join(ROOT, dirname(manifestRel));
const logos = [
  ...[...body.matchAll(/\b(Square\d+x\d+Logo|Wide\d+x\d+Logo)="([^"]+)"/g)].map((m) => [m[1], m[2]]),
  ...[...body.matchAll(/<Logo>([^<]+)<\/Logo>/g)].map((m) => ['Logo', m[1]]),
];
if (logos.length < 6) err(`manifest names ${logos.length} logos, want 6`);
for (const [attr, rel] of logos) {
  if (!LOGO_SIZES[attr]) err(`manifest logo attribute ${attr} has no known size`);
  else checkLogo(manifestDir, rel, LOGO_SIZES[attr], attr === 'Square44x44Logo');
}
const identity = /<Identity\b[\s\S]*?\/>/.exec(body)?.[0] ?? '';
if (!/Version="\d+\.\d+\.\d+\.0"/.test(identity)) err('manifest Identity Version must be x.y.z.0');

// --------------------------------------------------------------------- macOS

const plist = read('packaging/macos/Info.plist').replace(/<!--[\s\S]*?-->/g, '');
const extBlock = /<key>CFBundleTypeExtensions<\/key>\s*<array>([\s\S]*?)<\/array>/.exec(plist)?.[1] ?? '';
const plistExts = new Set([...extBlock.matchAll(/<string>([^<]+)<\/string>/g)].map((m) => m[1]));
if (!setEq(plistExts, exts)) {
  err(`Info.plist extensions differ from extensions.txt: missing [${diff(exts, plistExts)}] extra [${diff(plistExts, exts)}]`);
}
for (const key of ['CFBundleExecutable', 'CFBundleIdentifier', 'CFBundleIconFile', 'CFBundleShortVersionString']) {
  if (!plist.includes(`<key>${key}</key>`)) err(`Info.plist lacks ${key}`);
}
if (!existsSync(join(ROOT, 'packaging/macos/pandite.icns'))) err('packaging/macos/pandite.icns is missing');
else if (readFileSync(join(ROOT, 'packaging/macos/pandite.icns')).toString('latin1', 0, 4) !== 'icns') err('pandite.icns is not an icns file');

// --------------------------------------------------------------------- Linux

const desktop = read('packaging/linux/pandite.desktop');
const mimeLine = /^MimeType=(.*)$/m.exec(desktop)?.[1] ?? '';
const mimes = mimeLine.split(';').filter(Boolean);
const linuxExts = new Set();
for (const m of mimes) {
  if (!MIME_EXT[m]) err(`pandite.desktop: MIME type ${m} has no extension mapping in check.mjs`);
  else for (const e of MIME_EXT[m]) linuxExts.add(e);
}
const linuxWant = new Set([...exts].filter((e) => !LINUX_EXCLUDED.has(e)));
if (!setEq(linuxExts, linuxWant)) {
  err(`pandite.desktop MIME types differ: missing [${diff(linuxWant, linuxExts)}] extra [${diff(linuxExts, linuxWant)}]`);
}
for (const key of ['Type=Application', 'Name=pandite', 'Exec=pandite %f', 'Icon=pandite']) {
  if (!desktop.split(/\r?\n/).includes(key)) err(`pandite.desktop lacks "${key}"`);
}
for (const n of [16, 32, 48, 64, 128, 256, 512]) {
  const p = join(ROOT, `packaging/linux/icons/hicolor/${n}x${n}/apps/pandite.png`);
  const size = existsSync(p) ? pngSize(p) : null;
  if (!size || size[0] !== n || size[1] !== n) err(`Linux icon ${n}x${n} is missing or has the wrong size`);
}

// ----------------------------------------------------------------- workflows

for (const wf of ['.github/workflows/ci.yml', '.github/workflows/release.yml']) {
  if (!existsSync(join(ROOT, wf))) {
    err(`${wf} is missing`);
    continue;
  }
  const text = read(wf);
  for (const m of text.matchAll(/uses:\s*([^\s#]+)/g)) {
    if (!/^[\w.-]+\/[\w.-]+@v\d+$/.test(m[1])) err(`${wf}: action ${m[1]} is not pinned to a major version (owner/name@vN)`);
  }
  for (const m of text.matchAll(/\b((?:packaging|scripts)\/[\w./-]+\.(?:mjs|sh|ps1))\b/g)) {
    if (!existsSync(join(ROOT, m[1]))) err(`${wf} calls ${m[1]}, which does not exist`);
  }
  if (!/^permissions:/m.test(text)) err(`${wf} sets no top-level permissions`);
  if (!/^concurrency:/m.test(text)) err(`${wf} sets no concurrency group`);
}

// --------------------------------------------------------------------- ASCII

const TEXT_EXT = /\.(mjs|js|sh|ps1|xml|plist|desktop|txt|md|yml|yaml)$/i;
function walk(rel) {
  const abs = join(ROOT, rel);
  if (!existsSync(abs)) return [];
  if (statSync(abs).isFile()) return [rel];
  return readdirSync(abs).flatMap((n) => walk(join(rel, n)));
}
const textFiles = ['packaging', '.github', 'docs', 'README.md'].flatMap(walk).filter((f) => TEXT_EXT.test(f));
for (const f of textFiles) {
  const bytes = readFileSync(join(ROOT, f));
  const bad = bytes.findIndex((b) => b > 0x7e || (b < 0x20 && b !== 0x0a && b !== 0x0d && b !== 0x09));
  if (bad >= 0) {
    const line = bytes.subarray(0, bad).toString('latin1').split('\n').length;
    err(`${f}:${line}: byte 0x${bytes[bad].toString(16)} is not printable ASCII`);
  }
  if (/\.ya?ml$/.test(f) && bytes.includes(0x09)) err(`${f}: YAML must not contain tabs`);
}

if (errors.length) {
  for (const e of errors) console.error(`STEP FAILED: ${e}`);
  process.exit(1);
}
console.log(`${exts.size} extensions in manifest, Info.plist and desktop entry; ${logos.length} logos with variants; ${textFiles.length} text files ASCII`);
console.log('PACKAGING CONSISTENT');
