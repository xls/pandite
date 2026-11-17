#!/usr/bin/env node
// Generate docs/THIRD-PARTY-NOTICES.md from `cargo metadata`.
//
// Usage: node packaging/third-party-notices.mjs [--check]
//
// Lists every crate linked into the pandite binary on the five release
// targets (normal dependencies of pandite-app; build scripts and procedural
// macros run at compile time and are left out), with its license and the
// license, notice and copyright files shipped in the crate. Identical texts
// are printed once. Files embedded in the program that are not crates (the
// interface font, the text recognition models) get sections of their own.
// The output is ASCII. --check compares instead of writing and exits 1 when
// the committed file is out of date.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'docs', 'THIRD-PARTY-NOTICES.md');
const FONT_LICENSE = join(ROOT, 'crates', 'pandite-render', 'fonts', 'FiraSans-OFL.txt');
// Files embedded in the program that are not crates. Each gets its
// attribution under "Required attributions" and a section with its notice
// and license text. The notice must name the SHA-256 of every file.
const EMBEDDED = [
  {
    title: 'PP-OCRv6 text recognition models',
    systems: 'macOS and Linux programs',
    files: [
      'crates/pandite-ocr/models/PP-OCRv6_det_tiny.onnx',
      'crates/pandite-ocr/models/PP-OCRv6_rec_tiny.onnx',
    ],
    attribution: [
      'The text recognition of the macOS and Linux programs uses the PP-OCRv6 tiny models',
      'of PaddleOCR, as converted to ONNX by RapidOCR. The model weights are copyright Baidu',
      'and/or the applicable PaddleOCR rights holders, the conversion is copyright the RapidOCR',
      'Authors, and both are licensed under the Apache License, Version 2.0. The notice and the',
      'full license are in the section "PP-OCRv6 text recognition models" below.',
    ],
    notice: 'crates/pandite-ocr/models/NOTICE',
    license: 'crates/pandite-ocr/models/LICENSE-APACHE',
  },
];
const TARGETS = [
  ['x86_64-pc-windows-msvc', 'Windows'],
  ['aarch64-pc-windows-msvc', 'Windows'],
  ['x86_64-apple-darwin', 'macOS'],
  ['aarch64-apple-darwin', 'macOS'],
  ['x86_64-unknown-linux-gnu', 'Linux'],
];
const LICENSE_FILE = /^(licen[cs]e|copying|copyright|notice|unlicense)([-._].*)?$/i;

function fail(message) {
  console.error(`STEP FAILED: ${message}`);
  process.exit(1);
}

function toolEnv() {
  const env = { ...process.env };
  env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
  delete env.Path;
  return env;
}

function metadata(target) {
  const args = ['metadata', '--format-version', '1', '--locked', '--filter-platform', target];
  const res = spawnSync('cargo', args, { env: toolEnv(), cwd: ROOT, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  if (res.error || res.status !== 0) fail(`cargo ${args.join(' ')}\n${res.stderr ?? res.error?.message}`);
  return JSON.parse(res.stdout);
}

/** The cfg values of a target (`unix`, `target_os="linux"`, ...) from rustc. */
function targetCfg(target) {
  const res = spawnSync('rustc', ['--print', 'cfg', '--target', target], { env: toolEnv(), encoding: 'utf8' });
  if (res.error || res.status !== 0) fail(`rustc --print cfg --target ${target}\n${res.stderr ?? res.error?.message}`);
  return new Set(res.stdout.split(/\r?\n/).map((l) => l.trim()).filter(Boolean));
}

/**
 * True when a dependency's platform (`null`, a target triple or a `cfg(...)`
 * expression) holds for `target`. `--filter-platform` keeps a dependency
 * when any of its kinds matches, so a normal kind for other systems can
 * come along with a dev kind for all of them.
 */
function platformMatches(spec, target, cfg) {
  if (spec === null || spec === undefined) return true;
  if (!spec.startsWith('cfg(')) return spec === target;
  let i = 4;
  const bad = () => fail(`cannot read the platform ${spec}`);
  const ws = () => { while (spec[i] === ' ') i += 1; };
  const expr = () => {
    ws();
    const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(spec.slice(i));
    if (!m) bad();
    const name = m[0];
    i += name.length;
    ws();
    if (['all', 'any', 'not'].includes(name) && spec[i] === '(') {
      i += 1;
      const args = [];
      ws();
      while (spec[i] !== ')') {
        if (i >= spec.length) bad();
        args.push(expr());
        ws();
        if (spec[i] === ',') i += 1;
        ws();
      }
      i += 1;
      if (name === 'all') return args.every(Boolean);
      if (name === 'any') return args.some(Boolean);
      if (args.length !== 1) bad();
      return !args[0];
    }
    if (spec[i] === '=') {
      i += 1;
      ws();
      const v = /^"([^"]*)"/.exec(spec.slice(i));
      if (!v) bad();
      i += v[0].length;
      return cfg.has(`${name}="${v[1]}"`);
    }
    return cfg.has(name);
  };
  const value = expr();
  ws();
  if (spec[i] !== ')' || i !== spec.length - 1) bad();
  return value;
}

/** Package ids linked into pandite-app for one target. */
function linked(meta, target, cfg) {
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const root = meta.packages.find((p) => p.name === 'pandite-app');
  if (!root) fail('pandite-app is not in the workspace');
  const seen = new Set();
  const stack = [root.id];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) {
      if (!dep.dep_kinds.some((k) => k.kind === null && platformMatches(k.target, target, cfg))) continue;
      const pkg = byId.get(dep.pkg);
      if (pkg.targets.some((t) => t.kind.includes('proc-macro'))) continue;
      stack.push(dep.pkg);
    }
  }
  // Registry crates and patched copies in vendor/; not the workspace crates.
  const own = new Set(meta.workspace_members);
  return [...seen].map((id) => byId.get(id)).filter((p) => !own.has(p.id));
}

// Code point -> ASCII spelling for symbols that NFKD does not reduce.
const ASCII_MAP = new Map([
  [0xa9, '(c)'], [0xae, '(R)'], [0x2122, '(TM)'], [0x2018, "'"], [0x2019, "'"], [0x201a, "'"],
  [0x201c, '"'], [0x201d, '"'], [0x201e, '"'], [0x2013, '-'], [0x2014, '-'], [0x2212, '-'],
  [0x2026, '...'], [0x2022, '*'], [0xa0, ' '], [0xdf, 'ss'], [0xe6, 'ae'], [0xc6, 'AE'],
  [0xf8, 'o'], [0xd8, 'O'], [0x142, 'l'], [0x141, 'L'], [0x111, 'd'], [0x110, 'D'],
  [0xb7, '*'], [0xab, '"'], [0xbb, '"'], [0xfeff, ''],
]);

/** Plain ASCII: accents dropped, typographic symbols spelled out. */
function ascii(text) {
  return text
    .replace(/\r\n?/g, '\n')
    .replace(/\t/g, '    ')
    .normalize('NFKD')
    .replace(/\p{M}/gu, '')
    .replace(/[^\x0a\x20-\x7e]/g, (c) => ASCII_MAP.get(c.codePointAt(0)) ?? '?');
}

function licenseFiles(pkg) {
  const dir = dirname(pkg.manifest_path);
  const files = new Set();
  for (const name of readdirSync(dir)) {
    if (LICENSE_FILE.test(name) && statSync(join(dir, name)).isFile()) files.add(name);
  }
  if (pkg.license_file && existsSync(join(dir, pkg.license_file))) files.add(pkg.license_file);
  return [...files].sort().map((name) => ({ name, text: ascii(readFileSync(join(dir, name), 'utf8')).trim() }));
}

/** Locale-independent order, so every machine writes the same file. */
function byCodeUnit(a, b) {
  if (a < b) return -1;
  return a > b ? 1 : 0;
}

function render() {
  const crates = new Map();
  for (const [target, os] of TARGETS) {
    for (const pkg of linked(metadata(target), target, targetCfg(target))) {
      const key = `${pkg.name} ${pkg.version}`;
      if (!crates.has(key)) crates.set(key, { pkg, systems: new Set() });
      crates.get(key).systems.add(os);
    }
  }
  const list = [...crates.values()].sort((a, b) => (a.pkg.name === b.pkg.name
    ? byCodeUnit(a.pkg.version, b.pkg.version)
    : byCodeUnit(a.pkg.name, b.pkg.name)));

  const lines = [];
  lines.push('# Third-party notices');
  lines.push('');
  lines.push('pandite is released under the MIT License (see `LICENSE`). The pandite program');
  lines.push('includes the third-party components listed below. Each component keeps its own');
  lines.push('license. `node packaging/third-party-notices.mjs` generates this file from');
  lines.push('`cargo metadata` for the five release targets. Do not edit it by hand.');
  lines.push('');
  lines.push('## Required attributions');
  lines.push('');
  lines.push('This software is based in part on the work of the Independent JPEG Group.');
  lines.push('(The JPEG encoder, crate `jpeg-encoder`, contains a forward DCT ported from the');
  lines.push('IJG code. The full IJG terms are in the `jpeg-encoder` license texts below.)');
  lines.push('');
  lines.push('The interface font is a subset of Fira Sans Regular, Copyright (c) 2012-2015,');
  lines.push('The Mozilla Foundation and Telefonica S.A., licensed under the SIL Open Font');
  lines.push('License, Version 1.1. The full license is in the section "Fira Sans" below.');
  lines.push('');
  for (const item of EMBEDDED) {
    lines.push(...item.attribution);
    lines.push('');
  }
  lines.push('## Components');
  lines.push('');
  lines.push(`${list.length} crates. "All" means every release target.`);
  lines.push('');
  lines.push('| Crate | Version | License | Systems |');
  lines.push('|---|---|---|---|');
  for (const { pkg, systems } of list) {
    const sys = systems.size === 3 ? 'All' : [...systems].sort().join(', ');
    lines.push(`| ${pkg.name} | ${pkg.version} | ${ascii(pkg.license ?? 'see license file')} | ${sys} |`);
  }
  lines.push('');
  lines.push('## Fira Sans');
  lines.push('');
  lines.push('Embedded in the program as `crates/pandite-render/fonts/FiraSans-Regular-subset.ttf`.');
  lines.push('');
  lines.push('```text');
  lines.push(ascii(readFileSync(FONT_LICENSE, 'utf8')).trim());
  lines.push('```');
  lines.push('');
  for (const item of EMBEDDED) {
    const notice = ascii(readFileSync(join(ROOT, item.notice), 'utf8')).trim();
    for (const file of item.files) {
      const path = join(ROOT, file);
      if (!existsSync(path)) fail(`${file} is missing`);
      const sha = createHash('sha256').update(readFileSync(path)).digest('hex');
      if (!notice.includes(sha)) fail(`${item.notice} does not name the SHA-256 ${sha} of ${file}`);
    }
    lines.push(`## ${item.title}`);
    lines.push('');
    lines.push(`Embedded in the ${item.systems} as ${item.files.map((f) => `\`${f}\``).join(' and ')}.`);
    lines.push('');
    lines.push('```text');
    lines.push(notice.replace(/```/g, "'''"));
    lines.push('```');
    lines.push('');
    lines.push('```text');
    lines.push(ascii(readFileSync(join(ROOT, item.license), 'utf8')).trim().replace(/```/g, "'''"));
    lines.push('```');
    lines.push('');
  }
  lines.push('## License texts');
  lines.push('');
  lines.push('Each text below is printed once, with every crate that ships it.');
  lines.push('');

  const groups = new Map();
  const missing = [];
  for (const { pkg } of list) {
    const files = licenseFiles(pkg);
    if (files.length === 0) missing.push(pkg);
    for (const f of files) {
      const key = f.text.replace(/\s+/g, ' ');
      if (!groups.has(key)) groups.set(key, { text: f.text, users: [] });
      groups.get(key).users.push(`${pkg.name} ${pkg.version} (${f.name})`);
    }
  }
  const sorted = [...groups.values()].sort((a, b) => byCodeUnit(a.users[0], b.users[0]));
  sorted.forEach((g, i) => {
    lines.push(`### Text ${i + 1}`);
    lines.push('');
    lines.push(`Used by: ${g.users.join(', ')}.`);
    lines.push('');
    lines.push('```text');
    lines.push(g.text.replace(/```/g, "'''"));
    lines.push('```');
    lines.push('');
  });
  if (missing.length) {
    lines.push('### Crates without a license file in the published package');
    lines.push('');
    lines.push('These crates state their license in Cargo.toml only. The standard texts of those');
    lines.push('licenses are among the texts above.');
    lines.push('');
    for (const pkg of missing) {
      lines.push(`- ${pkg.name} ${pkg.version}: ${ascii(pkg.license ?? 'unknown')}${pkg.repository ? `, ${pkg.repository}` : ''}`);
    }
    lines.push('');
  }
  return `${lines.join('\n').trimEnd()}\n`;
}

const text = render();
if (process.argv.includes('--check')) {
  const current = existsSync(OUT) ? readFileSync(OUT, 'utf8').replace(/\r\n/g, '\n') : '';
  if (current !== text) {
    console.error('docs/THIRD-PARTY-NOTICES.md is out of date; run node packaging/third-party-notices.mjs');
    process.exit(1);
  }
  console.log('THIRD-PARTY NOTICES UP TO DATE');
} else {
  writeFileSync(OUT, text);
  console.log(`wrote ${OUT}`);
}
