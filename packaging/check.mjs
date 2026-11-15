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
//  - packaging/macos/pandite.entitlements is plist text that turns on the
//    App Sandbox with exactly the file and bookmark entitlements the code
//    needs (no network), and build-app.sh signs with it only in its store
//    and sandbox test modes, keeping the default Developer ID signature
//  - packaging/linux/pandite.desktop covers the same extensions through its
//    MIME types, minus the formats Linux cannot decode
//  - every packaging script the workflows call exists, and every action is
//    pinned to a major version
//  - the Windows setup script (pandite.iss) takes its version from the
//    build, installs the files of the portable zip per user, runs
//    --register, --quit and --unregister, has the right architectures, and
//    names the publisher and URL of LICENSE and Cargo.toml
//  - the Debian control template and the RPM spec take their version from
//    the build, ship the files of the tarball (with its desktop entry, so
//    the same MIME types), and recommend the same run-time libraries
//  - the npm installer has the bin names pandite and pandite-installer, a
//    files list, no dependencies and no install script; its repository,
//    setup AppId and release file names match the build scripts and
//    pandite.iss; no file of the old npm launcher is left
//  - the release workflow passes the version to every package builder,
//    pins the Inno Setup download, and neither builds nor publishes an npm
//    package; ci.yml builds no installer
//  - packaging, workflow and documentation text files are pure ASCII
//    without tabs in YAML

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

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

// The App Sandbox entitlements of the store and sandbox test builds.
const entRel = 'packaging/macos/pandite.entitlements';
const ENTITLEMENTS = [
  'com.apple.security.app-sandbox',
  'com.apple.security.files.user-selected.read-write',
  'com.apple.security.files.bookmarks.app-scope',
];
if (!existsSync(join(ROOT, entRel))) err(`${entRel} is missing`);
else {
  const ent = read(entRel).replace(/<!--[\s\S]*?-->/g, '');
  const head = /^<\?xml version="1\.0" encoding="UTF-8"\?>\s*<!DOCTYPE plist PUBLIC "-\/\/Apple\/\/DTD PLIST 1\.0\/\/EN" "http:\/\/www\.apple\.com\/DTDs\/PropertyList-1\.0\.dtd">\s*/.exec(ent);
  const body = head ? ent.slice(head[0].length) : '';
  const dict = /^<plist version="1\.0">\s*<dict>((?:\s*<key>[^<]+<\/key>\s*(?:<true\/>|<false\/>))*)\s*<\/dict>\s*<\/plist>\s*$/.exec(body);
  if (!dict) err(`${entRel} is not a plist of keys with true or false values`);
  else {
    const entries = [...dict[1].matchAll(/<key>([^<]+)<\/key>\s*<(true|false)\/>/g)].map((m) => [m[1], m[2]]);
    const keys = new Set(entries.map(([k]) => k));
    if (keys.size !== entries.length) err(`${entRel} names a key twice`);
    if (!setEq(keys, new Set(ENTITLEMENTS))) err(`${entRel} must hold exactly [${ENTITLEMENTS}], found [${[...keys]}]`);
    for (const [k, v] of entries) if (v !== 'true') err(`${entRel}: ${k} must be true`);
    if (!entries.some(([k, v]) => k === 'com.apple.security.app-sandbox' && v === 'true')) err(`${entRel} does not turn on the App Sandbox`);
    if ([...keys].some((k) => /network/.test(k))) err(`${entRel} has a network entitlement`);
  }
}
const buildApp = read('packaging/macos/build-app.sh');
for (const [what, line] of [
  ['the entitlements file', 'ENTITLEMENTS="$HERE/pandite.entitlements"'],
  ['the store mode only with PANDITE_MACOS_STORE=1', 'if [ "${PANDITE_MACOS_STORE:-}" = 1 ]; then'],
  ['the sandbox test mode only with PANDITE_MACOS_SANDBOX_TEST=1', 'if [ "${PANDITE_MACOS_SANDBOX_TEST:-}" = 1 ]; then'],
  ['the Developer ID signature without entitlements', 'codesign --force --options runtime --timestamp --sign "$IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$APP"'],
  ['the ad hoc signature without entitlements', 'codesign --force --sign - --timestamp=none "$APP"'],
  ['the provisioning profile in the store app', 'cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"'],
  ['the store package', 'productbuild --component "$APP" /Applications --sign "$INSTALLER_IDENTITY"'],
  ['the store package name', 'PKG="$OUT/pandite-$VERSION-macos-appstore.pkg"'],
]) {
  if (!buildApp.includes(line)) err(`build-app.sh lacks ${what}: ${line}`);
}
const entitled = buildApp.split(/\r?\n/).filter((l) => /^\s*codesign\b.*--entitlements/.test(l));
if (entitled.length !== 3) err(`build-app.sh signs with entitlements on ${entitled.length} lines, want 3 (store, sandbox test with and without an identity)`);

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
for (const key of ['Type=Application', 'Name=Pandite', 'Exec=pandite %f', 'Icon=pandite']) {
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

// ------------------------------------------------------- version and names

const cargo = read('Cargo.toml');
const workspace = (cargo.split(/^\[workspace\.package\]\s*$/m)[1] ?? '').split(/^\[/m)[0];
const cargoField = (key) => new RegExp(`^${key}\\s*=\\s*"([^"]+)"`, 'm').exec(workspace)?.[1];
const VERSION = cargoField('version');
const REPO = cargoField('repository');
const LICENSE_ID = cargoField('license');
if (!VERSION || !REPO || !LICENSE_ID) err('Cargo.toml [workspace.package] lacks version, repository or license');
const HOLDER = /^Copyright \(c\) \d{4}(?:-\d{4})? (.+)$/m.exec(read('LICENSE'))?.[1]?.trim();
if (!HOLDER) err('LICENSE has no "Copyright (c) <year> <name>" line');
const release = read('.github/workflows/release.yml');
const ci = read('.github/workflows/ci.yml');

// Files of the portable zip, as build-zip.ps1 stages them.
const zipFiles = new Set([...read('packaging/windows/build-zip.ps1').matchAll(/Join-Path \$stage '([^']+)'/g)].map((m) => m[1]));
if (!zipFiles.has('pandite.exe')) err('build-zip.ps1 stages no pandite.exe');

// --------------------------------------------------- Windows setup program

/** Sections of an Inno Setup script: name -> lines without comments. */
function issSections(text) {
  const sections = new Map();
  let current = '';
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const head = /^\[(\w+)\]$/.exec(line);
    if (head) {
      current = head[1];
      if (!sections.has(current)) sections.set(current, []);
      continue;
    }
    if (!current || !line || line.startsWith(';') || line.startsWith('#')) continue;
    sections.get(current).push(line);
  }
  return sections;
}
/** Parameters of an Inno Setup entry line: Name: "x"; Flags: a b. */
function issEntry(line) {
  const out = {};
  for (const m of line.matchAll(/(\w+):\s*("(?:[^"]|"")*"|[^;]*)/g)) {
    const v = m[2].trim();
    out[m[1]] = v.startsWith('"') ? v.slice(1, -1).replace(/""/g, '"') : v;
  }
  return out;
}

const issRel = 'packaging/windows/pandite.iss';
const iss = read(issRel);
const issDir = join(ROOT, 'packaging/windows');
const sections = issSections(iss);
const setup = {};
for (const line of sections.get('Setup') ?? []) {
  const m = /^(\w+)=(.*)$/.exec(line);
  if (m) (setup[m[1]] ??= []).push(m[2]);
}
const one = (key) => (setup[key]?.length === 1 ? setup[key][0] : undefined);
const want = {
  AppName: 'Pandite',
  AppVersion: '{#Version}',
  VersionInfoVersion: '{#NumericVersion}',
  AppPublisher: HOLDER,
  AppPublisherURL: REPO,
  AppSupportURL: `${REPO}/issues`,
  AppUpdatesURL: `${REPO}/releases`,
  PrivilegesRequired: 'lowest',
  DefaultDirName: '{localappdata}\\Programs\\Pandite',
  UninstallDisplayIcon: '{app}\\pandite.exe',
  OutputBaseFilename: 'pandite-{#Version}-{#FileTarget}-setup',
  OutputDir: '{#OutDir}',
};
for (const [key, value] of Object.entries(want)) {
  if (one(key) !== value) err(`${issRel}: ${key} is ${setup[key] ?? 'missing'}, want ${value}`);
}
if (!/^\{\{[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}\}$/.test(one('AppId') ?? '')) {
  err(`${issRel}: AppId must be one fixed GUID written {{GUID}`);
}
if (/AppVersion=\d/.test(iss)) err(`${issRel}: the version must come from the build (/DVersion), not the script`);
const manifestMin = /TargetDeviceFamily[^>]*MinVersion="([\d.]+)"/.exec(body)?.[1]?.replace(/\.0$/, '');
if (one('MinVersion') !== manifestMin) err(`${issRel}: MinVersion ${one('MinVersion')} differs from the MSIX manifest ${manifestMin}`);
for (const [key, file, magic] of [
  ['SetupIconFile', 'crates/pandite-app/assets/pandite.ico', '00000100'],
  ['LicenseFile', 'LICENSE', null],
]) {
  const p = resolve(issDir, (one(key) ?? '').replace(/\\/g, '/'));
  if (relative(ROOT, p).replace(/\\/g, '/') !== file) err(`${issRel}: ${key} must be ${file}`);
  else if (magic && readFileSync(p).toString('hex', 0, 4) !== magic) err(`${file} is not an .ico file`);
}
// x64: x64 Windows and Windows 11 on ARM64. ARM64: ARM64 Windows only.
const archBlock = (arch) => new RegExp(`#(?:el)?if Arch == "${arch}"\\s*\\n(?:;.*\\n)*`).exec(iss);
for (const [arch, value] of [['x64', 'x64compatible'], ['arm64', 'arm64']]) {
  const at = archBlock(arch);
  const block = at ? iss.slice(at.index + at[0].length).split(/^#/m)[0] : '';
  if (!block.includes(`ArchitecturesAllowed=${value}\n`) || !block.includes(`ArchitecturesInstallIn64BitMode=${value}\n`)) {
    err(`${issRel}: the ${arch} build needs ArchitecturesAllowed and ArchitecturesInstallIn64BitMode = ${value}`);
  }
}
if (sections.has('Registry')) err(`${issRel}: file associations come from pandite.exe --register only, not from [Registry]`);
const issFiles = (sections.get('Files') ?? []).map(issEntry);
const issDest = new Set(issFiles.map((f) => f.DestName || basename(f.Source.replace(/\\/g, '/'))));
if (!setEq(issDest, zipFiles)) err(`${issRel} installs [${[...issDest].sort()}], the portable zip holds [${[...zipFiles].sort()}]`);
for (const f of issFiles) {
  if (f.DestDir !== '{app}') err(`${issRel}: ${f.Source} goes to ${f.DestDir}, want {app}`);
  if (!f.Source.startsWith('{#') && !existsSync(resolve(issDir, f.Source.replace(/\\/g, '/')))) err(`${issRel}: ${f.Source} does not exist`);
}
const tasks = new Map((sections.get('Tasks') ?? []).map(issEntry).map((t) => [t.Name, t]));
const assoc = tasks.get('associate');
if (assoc?.Description !== 'Associate image files with Pandite' || /unchecked/.test(assoc?.Flags ?? '')) {
  err(`${issRel}: task "associate" must read "Associate image files with Pandite" and be checked by default`);
}
if (!/unchecked/.test(tasks.get('desktopicon')?.Flags ?? '')) err(`${issRel}: the desktop shortcut task must be unchecked by default`);
const runs = (sections.get('Run') ?? []).map(issEntry);
if (!runs.some((r) => r.Filename === '{app}\\pandite.exe' && r.Parameters === '--register' && r.Tasks === 'associate')) {
  err(`${issRel}: [Run] must run {app}\\pandite.exe --register for the task associate`);
}
const unruns = (sections.get('UninstallRun') ?? []).map(issEntry);
if (!unruns.some((r) => r.Filename === '{app}\\pandite.exe' && r.Parameters === '--unregister' && r.RunOnceId)) {
  err(`${issRel}: [UninstallRun] must run {app}\\pandite.exe --unregister with a RunOnceId`);
}
const icons = (sections.get('Icons') ?? []).map(issEntry);
if (!icons.some((i) => i.Name === '{autoprograms}\\Pandite' && i.Filename === '{app}\\pandite.exe')) err(`${issRel}: no Start menu shortcut "Pandite"`);
if (!icons.some((i) => i.Name === '{autodesktop}\\Pandite' && i.Tasks === 'desktopicon')) err(`${issRel}: no optional desktop shortcut`);
const code = (iss.split(/^\[Code\]$/m)[1] ?? '');
if (!/Exec\(Exe, '--quit'/.test(code)) err(`${issRel}: [Code] must run pandite.exe --quit`);
if (!/function PrepareToInstall[\s\S]*?QuitResidentPandite;[\s\S]*?end;/.test(code)) err(`${issRel}: PrepareToInstall must end the resident pandite`);
if (!/usUninstall then\s+QuitResidentPandite;/.test(code)) err(`${issRel}: uninstall must end the resident pandite`);
const buildSetup = read('packaging/windows/build-setup.ps1');
for (const m of iss.matchAll(/^#ifndef (\w+)$/gm)) {
  if (!buildSetup.includes(`"/D${m[1]}=`)) err(`build-setup.ps1 does not pass /D${m[1]} to ${issRel}`);
}

// ------------------------------------------------------- Debian and RPM

// The tarball items and where each package puts them.
const LAYOUT = [
  // [tarball path, Debian package path (build-deb.sh), RPM path (pandite.spec %files)]
  ['pandite', '$PKG/usr/bin/pandite', '%{_bindir}/pandite'],
  ['share/applications/pandite.desktop', '$PKG/usr/share/applications/pandite.desktop', '%{_datadir}/applications/pandite.desktop'],
  ['share/icons/hicolor', '$PKG/usr/share/icons/hicolor', '%{_datadir}/icons/hicolor/*/apps/pandite.png'],
  ['LICENSE', '$DOC/copyright', '%{_datadir}/licenses/pandite/LICENSE'],
  ['README.md', '$DOC/README.md.gz', '%{_datadir}/doc/pandite/README.md'],
  ['THIRD-PARTY-NOTICES.md', '$DOC/THIRD-PARTY-NOTICES.md.gz', '%{_datadir}/doc/pandite/THIRD-PARTY-NOTICES.md'],
];
const TARBALL_ONLY = ['install.sh'];
// Debian package -> soname of each library the program loads at run time.
const RUNTIME_LIBS = {
  libvulkan1: 'libvulkan.so.1',
  libegl1: 'libEGL.so.1',
  libxkbcommon0: 'libxkbcommon.so.0',
  'libxkbcommon-x11-0': 'libxkbcommon-x11.so.0',
  'libwayland-client0': 'libwayland-client.so.0',
  'libwayland-cursor0': 'libwayland-cursor.so.0',
  'libwayland-egl1': 'libwayland-egl.so.1',
  'libx11-6': 'libX11.so.6',
  'libx11-xcb1': 'libX11-xcb.so.1',
  libxcursor1: 'libXcursor.so.1',
  libxi6: 'libXi.so.6',
  libxrandr2: 'libXrandr.so.2',
};

const tarScript = read('packaging/linux/build-tarball.sh');
const tarItems = new Set([...tarScript.matchAll(/^(?:install|cp)\b.*"\$D\/([^"]+)"\s*$/gm)].map((m) => m[1]));
const tarWant = new Set([...LAYOUT.map((l) => l[0]), ...TARBALL_ONLY]);
if (!setEq(tarItems, tarWant)) {
  err(`build-tarball.sh packs [${[...tarItems].sort()}]; check.mjs LAYOUT maps [${[...tarWant].sort()}]: update the .deb, the .rpm and LAYOUT together`);
}
const debScript = read('packaging/linux/build-deb.sh');
const spec = read('packaging/linux/pandite.spec');
const specFiles = (spec.split(/^%files\s*$/m)[1] ?? '').split(/^%\w+\s*$/m)[0];
const specInstall = (spec.split(/^%install\s*$/m)[1] ?? '').split(/^%(?:post|postun|files|check)\b/m)[0];
for (const [tar, deb, rpm] of LAYOUT) {
  if (!debScript.includes(`"$SRC/${tar}"`) || !debScript.includes(deb)) err(`build-deb.sh does not install ${tar} as ${deb}`);
  if (!specInstall.includes(`"$src/${tar}`) && !specInstall.includes(`"$src"/${tar}`)) err(`pandite.spec %install does not take ${tar} from the tarball`);
  if (!specFiles.split(/\r?\n/).some((l) => l.trim().split(/\s+/).pop() === rpm)) err(`pandite.spec %files does not list ${rpm}`);
}
for (const [name, text] of [['build-deb.sh', debScript], ['pandite.spec', spec]]) {
  if (/MimeType|\.desktop"\s*\|?\s*sed|sed[^\n]*pandite\.desktop/.test(text)) err(`${name} must ship the desktop entry of the tarball unchanged (same MIME types)`);
}

const controlRel = 'packaging/linux/debian/control';
const control = read(controlRel);
const field = (key) => new RegExp(`^${key}: (.*)$`, 'm').exec(control)?.[1];
for (const [key, value] of [
  ['Package', 'pandite'],
  ['Version', '@VERSION@'],
  ['Architecture', 'amd64'],
  ['Installed-Size', '@INSTALLED_SIZE@'],
  ['Depends', '@DEPENDS@'],
  ['Homepage', REPO],
]) {
  if (field(key) !== value) err(`${controlRel}: ${key} is ${field(key)}, want ${value}`);
}
if (!(field('Maintainer') ?? '').startsWith(`${HOLDER} <`)) err(`${controlRel}: Maintainer must be "${HOLDER} <address>"`);
if (!field('Section') || !field('Description')) err(`${controlRel} lacks Section or Description`);
for (const token of control.matchAll(/@([A-Z_]+)@/g)) {
  if (!debScript.includes(`s|@${token[1]}@|`)) err(`build-deb.sh does not fill @${token[1]}@ in ${controlRel}`);
}
const debRecommends = (field('Recommends') ?? '').split(',').map((s) => s.trim()).filter((s) => s && !s.includes('|'));
if (!setEq(new Set(debRecommends), new Set(Object.keys(RUNTIME_LIBS)))) {
  err(`${controlRel}: Recommends [${debRecommends}] differs from the run-time libraries [${Object.keys(RUNTIME_LIBS)}]`);
}
const rpmRecommends = new Set([...spec.matchAll(/^Recommends:\s*(\S+)$/gm)].map((m) => m[1]));
const rpmWant = new Set(Object.values(RUNTIME_LIBS).map((so) => `${so}()(64bit)`));
if (!setEq(rpmRecommends, rpmWant)) err(`pandite.spec Recommends differs from the Debian Recommends: missing [${diff(rpmWant, rpmRecommends)}] extra [${diff(rpmRecommends, rpmWant)}]`);
for (const script of ['postinst', 'postrm']) {
  const text = read(`packaging/linux/debian/${script}`);
  if (!text.startsWith('#!/bin/sh\n') || !/^set -e$/m.test(text)) err(`debian/${script} must start with #!/bin/sh and set -e`);
  if (!text.includes('update-desktop-database') || !text.includes('gtk-update-icon-cache')) err(`debian/${script} must refresh the desktop and icon caches`);
}
const specField = (key) => new RegExp(`^${key}:\\s*(.+)$`, 'm').exec(spec)?.[1]?.trim();
for (const [key, value] of [
  ['Name', 'pandite'],
  ['Version', '%{pandite_version}'],
  ['License', LICENSE_ID],
  ['URL', REPO],
  ['BuildArch', 'x86_64'],
]) {
  if (specField(key) !== value) err(`pandite.spec: ${key} is ${specField(key)}, want ${value}`);
}
const rpmScript = read('packaging/linux/build-rpm.sh');
for (const macro of ['pandite_version', 'pandite_source']) {
  if (!rpmScript.includes(`--define "${macro} `)) err(`build-rpm.sh does not define ${macro}`);
}

// ----------------------------------------------------------- npm installer

// The npm package is an installer with its own version: it finds the
// releases on GitHub at run time and is not built by the release workflow.
const npmDir = 'packaging/npm';
const npmRel = `${npmDir}/package.json`;
const npmPkg = JSON.parse(read(npmRel));
if (npmPkg.name !== 'pandite') err(`${npmRel}: name must be pandite`);
if (!/^\d+\.\d+\.\d+$/.test(npmPkg.version ?? '')) err(`${npmRel}: version must be x.y.z (the installer version, independent of Cargo.toml)`);
if (npmPkg.license !== LICENSE_ID) err(`${npmRel}: license must be ${LICENSE_ID}`);
if (npmPkg.homepage !== REPO || !String(npmPkg.repository?.url ?? '').includes(REPO.replace(/^https:\/\//, ''))) err(`${npmRel}: homepage and repository must name ${REPO}`);
if (npmPkg.type !== 'module') err(`${npmRel}: type must be module`);
if (npmPkg.engines?.node !== '>=18') err(`${npmRel}: engines.node must be >=18`);
for (const key of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies', 'bundleDependencies', 'bundledDependencies']) {
  if (key in npmPkg) err(`${npmRel}: the installer has no ${key}`);
}
const INSTALL_SCRIPTS = ['preinstall', 'install', 'postinstall', 'preprepare', 'prepare', 'postprepare', 'prepack', 'postpack'];
for (const key of INSTALL_SCRIPTS) if (npmPkg.scripts?.[key] !== undefined) err(`${npmRel}: the installer has no ${key} script`);
if (existsSync(join(ROOT, npmDir, 'binding.gyp'))) err(`${npmDir}/binding.gyp makes npm build at install time`);
const npmBins = Object.entries(npmPkg.bin ?? {});
if (npmBins.map(([name]) => name).sort().join(',') !== 'pandite,pandite-installer') err(`${npmRel}: bin must be exactly pandite and pandite-installer`);
if (new Set(npmBins.map(([, file]) => file)).size !== 1) err(`${npmRel}: pandite and pandite-installer must run the same file`);
for (const [name, file] of npmBins) {
  if (!existsSync(join(ROOT, npmDir, file))) err(`${npmRel}: bin.${name} names ${file}, which does not exist`);
  else if (!read(`${npmDir}/${file}`).startsWith('#!/usr/bin/env node\n')) err(`${npmDir}/${file} must start with #!/usr/bin/env node`);
}
if (!Array.isArray(npmPkg.files) || npmPkg.files.slice().sort().join(',') !== 'bin/,lib/') err(`${npmRel}: files must be exactly bin/ and lib/ (npm adds package.json, README.md and LICENSE)`);
if (!existsSync(join(ROOT, npmDir, 'LICENSE')) || read(`${npmDir}/LICENSE`) !== read('LICENSE')) err(`${npmDir}/LICENSE must be a copy of LICENSE`);
for (const old of ['build.mjs', 'checksums.json', 'lib/launcher.js']) {
  if (existsSync(join(ROOT, npmDir, old))) err(`${npmDir}/${old} belongs to the old npm launcher; delete it`);
}
const installer = await import(pathToFileURL(join(ROOT, npmDir, 'lib/platform.js')).href);
if (installer.REPO_URL !== REPO) err(`${npmDir}/lib/platform.js reads the releases of ${installer.REPO_URL}, want ${REPO}`);
if (`{${installer.WINDOWS_APP_ID}` !== one('AppId')) err(`${npmDir}/lib/platform.js has the AppId ${installer.WINDOWS_APP_ID}; ${issRel} has ${one('AppId')}`);
if (installer.SUMS_FILE !== 'SHA256SUMS.txt' || !/sha256sum -- \* > SHA256SUMS\.txt/.test(release)) err(`the npm installer reads ${installer.SUMS_FILE}; release.yml must write SHA256SUMS.txt`);
// The release file names that the installer looks for, as the build scripts write them.
const appScript = read('packaging/macos/build-app.sh');
const installSh = read('packaging/linux/install.sh');
const winTargets = [...release.matchAll(/^\s*- target: (\S+-pc-windows-msvc)$/gm)].map((m) => m[1]);
const setupBase = one('OutputBaseFilename') ?? '';
const macBase = /^NAME="([^"]+)"$/m.exec(appScript)?.[1]?.replace('$VERSION', VERSION);
const tarTarget = /^TARGET=(\S+)$/m.exec(tarScript)?.[1];
const tarBase = /^NAME="([^"]+)"$/m.exec(tarScript)?.[1]?.replace('$VERSION', VERSION).replace('$TARGET', tarTarget);
const built = {
  setup: (name) =>
    // The short name of this build, and the long name of releases up to 0.2.2.
    winTargets.some(
      (t) =>
        name === `${setupBase.replace('{#Version}', VERSION).replace('{#FileTarget}', t.replace('-pc-windows-msvc', '-windows'))}.exe` ||
        name === `pandite-${VERSION}-${t}-setup.exe`,
    ) && buildSetup.includes("$fileTarget = $Target -replace '-pc-windows-msvc$', '-windows'"),
  // Releases up to 0.2.2 also carry the app as a zip. The installer takes
  // it there and takes the disk image in every later release.
  'app-zip': (name) => name === `pandite-${VERSION}-${installer.MAC_TARGET_OLD}.zip`,
  dmg: (name) =>
    (name === `${macBase}.dmg` || name === `pandite-${VERSION}-${installer.MAC_TARGET_OLD}.dmg`) &&
    macBase === `pandite-${VERSION}-${installer.MAC_TARGET}` &&
    appScript.includes(`APP="$WORK/${installer.MAC_APP}"`) &&
    appScript.includes('cp -R "$APP" "$DMG_SRC/"') &&
    appScript.includes('"$OUT/$NAME.dmg"'),
  tarball: (name) =>
    // The short name of this build, and the long name of releases up to 0.2.2.
    (name === `${tarBase}.tar.gz` || name === `pandite-${VERSION}-${installer.LINUX_TARGET_OLD}.tar.gz`) &&
    tarBase === installer.tarballTop(VERSION) &&
    tarScript.includes('"$OUT/$NAME.tar.gz"') &&
    tarScript.includes('-C "$WORK" -cf - "$NAME"') &&
    tarItems.has('install.sh') &&
    tarItems.has('pandite') &&
    installSh.includes('PREFIX=${PREFIX:-"$HOME/.local"}') &&
    installSh.includes('"--uninstall"'),
};
const installerKinds = new Set();
for (const p of installer.PLATFORMS) {
  for (const { name, kind } of installer.candidateAssets({ ...p, glibc: installer.MIN_GLIBC, version: VERSION })) {
    installerKinds.add(kind);
    if (!built[kind]?.(name)) err(`the npm installer looks for ${name} (${kind}) on ${p.platform} ${p.arch}, which no build script writes`);
  }
}
if (installerKinds.size !== Object.keys(built).length) err(`the npm installer uses the kinds [${[...installerKinds]}], check.mjs knows [${Object.keys(built)}]`);

// ---------------------------------------------------------- release steps

for (const [what, pattern] of [
  ['a setup program build with the release version', /build-setup\.ps1\s+-Exe "target\/\$env:TARGET\/release\/pandite\.exe"\s+-Version \$env:VERSION -Target \$env:TARGET/],
  ['a .deb build from the tarball with the release version', /build-deb\.sh "\$VERSION" dist\s*\\\s*"dist\/pandite-\$VERSION-x86_64-linux\.tar\.gz"/],
  ['an .rpm build from the tarball with the release version', /build-rpm\.sh "\$VERSION" dist\s*\\\s*"dist\/pandite-\$VERSION-x86_64-linux\.tar\.gz"/],
  ['a pinned Inno Setup 6 version (INNO_SETUP_VERSION)', /INNO_SETUP_VERSION: "6\.\d+\.\d+"/],
  ['the SHA-256 of the Inno Setup download (INNO_SETUP_SHA256)', /INNO_SETUP_SHA256: [0-9a-f]{64}\n/],
  ['release notes that name npx pandite', /- .*\\`npx pandite\\` installs/],
]) {
  if (!pattern.test(release)) err(`release.yml lacks ${what}`);
}
for (const [what, pattern] of [
  ['the NPM_TOKEN secret', /NPM_TOKEN/],
  ['an npm publish step', /npm publish/],
  ['an npm package build', /packaging\/npm\/build\.mjs|-npm\.tgz/],
]) {
  if (pattern.test(release)) err(`release.yml has ${what}; the npm installer is published by hand, apart from releases`);
}
if (/build-setup|pandite\.iss|ISCC|build-deb|build-rpm|rpmbuild|dpkg-deb|lintian|packaging\/npm/.test(ci)) {
  err('ci.yml builds installers or packages; the release workflow does that');
}

// --------------------------------------------------------------------- ASCII

const TEXT_EXT = /\.(mjs|js|sh|ps1|xml|plist|entitlements|desktop|txt|md|yml|yaml|iss|spec|json)$/i;
const isText = (f) => TEXT_EXT.test(f) || f.replace(/\\/g, '/').startsWith('packaging/linux/debian/');
function walk(rel) {
  const abs = join(ROOT, rel);
  if (!existsSync(abs)) return [];
  if (statSync(abs).isFile()) return [rel];
  return readdirSync(abs).flatMap((n) => walk(join(rel, n)));
}
const textFiles = ['packaging', '.github', 'docs', 'README.md'].flatMap(walk).filter(isText);
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
console.log(
  `version ${VERSION} from Cargo.toml for the setup program, .deb and .rpm; ` +
    `${issDest.size} setup files = portable zip; ${LAYOUT.length} tarball items in .deb and .rpm; ` +
    `npm installer ${npmPkg.version} finds the files of ${installer.PLATFORMS.length} systems`,
);
console.log('PACKAGING CONSISTENT');
