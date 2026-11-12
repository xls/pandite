#!/usr/bin/env node
// Generate every packaging image from the application icon.
//
// Usage: node packaging/make-assets.mjs [--icons <dir>]
//
// 1. Renders the icon at each needed pixel size through the app crate:
//      cargo run -p pandite-app --example make_icon -- <dir> <size>...
//    (--icons <dir> reuses icon-<size>.png files that already exist there).
// 2. Composes the logos on transparent canvases, without resampling:
//      packaging/windows/assets/*.png   MSIX logos, every scale and target size
//      packaging/macos/pandite.icns     macOS icon (PNG entries)
//      packaging/linux/icons/hicolor/<n>x<n>/apps/pandite.png
// The output is committed. Rerun this script after the icon changes.

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { decodePng, encodePng } from './png.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');

const SCALES = [100, 125, 150, 200, 400];
const TARGET_SIZES = [16, 24, 32, 48, 256];

// MSIX logos. `fill` is the icon side as a share of the shorter canvas side.
const WINDOWS_LOGOS = [
  { name: 'Square44x44Logo', w: 44, h: 44, fill: 1.0, targetSizes: true },
  { name: 'StoreLogo', w: 50, h: 50, fill: 1.0 },
  { name: 'SmallTile', w: 71, h: 71, fill: 0.6 },
  { name: 'Square150x150Logo', w: 150, h: 150, fill: 0.6 },
  { name: 'Wide310x150Logo', w: 310, h: 150, fill: 0.6 },
  { name: 'LargeTile', w: 310, h: 310, fill: 0.5 },
];

// macOS: the rounded tile sits inside the standard icon grid margin.
const MAC_FILL = 0.86;
const ICNS_ENTRIES = [
  ['icp4', 16], ['icp5', 32], ['icp6', 64], ['ic07', 128], ['ic08', 256], ['ic09', 512],
  ['ic10', 1024], ['ic11', 32], ['ic12', 64], ['ic13', 256], ['ic14', 512],
];
const LINUX_SIZES = [16, 24, 32, 48, 64, 128, 256, 512];

function fail(message) {
  console.error(`STEP FAILED: ${message}`);
  process.exit(1);
}

/** One output image: canvas size and the icon side drawn centered on it. */
function plan(w, h, fill) {
  return { w, h, icon: Math.max(1, Math.round(Math.min(w, h) * fill)) };
}

function windowsPlans() {
  const out = [];
  for (const logo of WINDOWS_LOGOS) {
    out.push({ file: `${logo.name}.png`, ...plan(logo.w, logo.h, logo.fill) });
    for (const scale of SCALES) {
      const w = Math.round((logo.w * scale) / 100);
      const h = Math.round((logo.h * scale) / 100);
      out.push({ file: `${logo.name}.scale-${scale}.png`, ...plan(w, h, logo.fill) });
    }
    if (logo.targetSizes) {
      for (const n of TARGET_SIZES) {
        for (const suffix of ['', '_altform-unplated', '_altform-lightunplated']) {
          out.push({ file: `${logo.name}.targetsize-${n}${suffix}.png`, ...plan(n, n, 1.0) });
        }
      }
    }
  }
  return out;
}

function macPlans() {
  return [...new Set(ICNS_ENTRIES.map(([, n]) => n))].map((n) => ({ size: n, ...plan(n, n, MAC_FILL) }));
}

function renderIcons(sizes, dir) {
  const env = { ...process.env };
  env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
  delete env.Path;
  env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(ROOT, 'target-packaging');
  env.CARGO_TERM_COLOR = 'never';
  const args = ['run', '-q', '-p', 'pandite-app', '--example', 'make_icon', '--', dir, ...sizes.map(String)];
  console.log(`cargo ${args.slice(0, 6).join(' ')} -- <dir> ${sizes.length} sizes`);
  const res = spawnSync('cargo', args, { env, cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (res.error || res.status !== 0) {
    const tail = `${res.stdout ?? ''}\n${res.stderr ?? ''}`.split(/\r?\n/).filter(Boolean).slice(-30).join('\n');
    fail(`make_icon failed\n${res.error ? res.error.message : ''}\n${tail}`);
  }
}

function loadIcon(dir, size) {
  const path = join(dir, `icon-${size}.png`);
  if (!existsSync(path)) fail(`missing ${path}`);
  const img = decodePng(readFileSync(path));
  if (img.width !== size || img.height !== size) fail(`${path} is ${img.width}x${img.height}, want ${size}x${size}`);
  return img;
}

/** A w x h transparent canvas with `icon` centered on whole pixels. */
function compose(w, h, icon) {
  const out = new Uint8Array(w * h * 4);
  const ox = Math.floor((w - icon.width) / 2);
  const oy = Math.floor((h - icon.height) / 2);
  for (let y = 0; y < icon.height; y += 1) {
    const src = icon.rgba.subarray(y * icon.width * 4, (y + 1) * icon.width * 4);
    out.set(src, ((oy + y) * w + ox) * 4);
  }
  return encodePng(w, h, out);
}

function icns(pngBySize) {
  const parts = ICNS_ENTRIES.map(([type, n]) => {
    const png = pngBySize.get(n);
    const head = Buffer.alloc(8);
    head.write(type, 0, 'latin1');
    head.writeUInt32BE(png.length + 8, 4);
    return Buffer.concat([head, png]);
  });
  const body = Buffer.concat(parts);
  const head = Buffer.alloc(8);
  head.write('icns', 0, 'latin1');
  head.writeUInt32BE(body.length + 8, 4);
  return Buffer.concat([head, body]);
}

function main() {
  const args = process.argv.slice(2);
  const iconsArg = args.indexOf('--icons');
  const win = windowsPlans();
  const mac = macPlans();
  const linux = LINUX_SIZES.map((n) => ({ size: n, ...plan(n, n, 1.0) }));
  const sizes = [...new Set([...win, ...mac, ...linux].map((p) => p.icon))].sort((a, b) => a - b);

  let dir;
  if (iconsArg >= 0) {
    dir = resolve(args[iconsArg + 1] ?? fail('--icons needs a folder'));
  } else {
    dir = join(ROOT, 'target-packaging', 'icons');
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    renderIcons(sizes, dir);
  }
  const icons = new Map(sizes.map((s) => [s, loadIcon(dir, s)]));

  const winDir = join(HERE, 'windows', 'assets');
  rmSync(winDir, { recursive: true, force: true });
  mkdirSync(winDir, { recursive: true });
  for (const p of win) writeFileSync(join(winDir, p.file), compose(p.w, p.h, icons.get(p.icon)));

  const macPng = new Map(mac.map((p) => [p.size, compose(p.w, p.h, icons.get(p.icon))]));
  writeFileSync(join(HERE, 'macos', 'pandite.icns'), icns(macPng));

  const linuxDir = join(HERE, 'linux', 'icons', 'hicolor');
  rmSync(linuxDir, { recursive: true, force: true });
  for (const p of linux) {
    const d = join(linuxDir, `${p.size}x${p.size}`, 'apps');
    mkdirSync(d, { recursive: true });
    writeFileSync(join(d, 'pandite.png'), compose(p.w, p.h, icons.get(p.icon)));
  }

  const count = readdirSync(winDir).length;
  console.log(`${count} MSIX logos, ${ICNS_ENTRIES.length} icns entries, ${linux.length} Linux icons from ${sizes.length} icon sizes`);
  console.log('ASSETS WRITTEN');
}

main();
