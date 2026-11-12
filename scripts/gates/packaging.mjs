#!/usr/bin/env node
// Gate oracle: Store packaging and CI definitions cover the contract.
//
// Usage: node scripts/gates/packaging.mjs
//
// Checks, from the files on disk:
//  - CI and release workflows exist and name every release target
//  - the release workflow builds an MSIX and uploads release assets
//  - the MSIX manifest is balanced XML, declares no network capability,
//    and associates every extension in packaging/extensions.txt
//  - every logo the manifest references exists and is a PNG
//  - packaging/extensions.txt equals the extension list the codecs crate
//    exports through `cargo run -p pandite-codecs --example extensions`

import { readFileSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join, delimiter, dirname } from 'node:path';
import process from 'node:process';

const fail = (msg) => {
  console.error(`STEP FAILED: ${msg}`);
  process.exit(1);
};
const read = (p) => (existsSync(p) ? readFileSync(p, 'utf8') : fail(`missing file: ${p}`));

const TARGETS = [
  'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'x86_64-apple-darwin',
  'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu',
];

const ci = read('.github/workflows/ci.yml');
const release = read('.github/workflows/release.yml');
for (const t of TARGETS) {
  if (!ci.includes(t)) fail(`ci.yml does not build ${t}`);
  if (!release.includes(t)) fail(`release.yml does not build ${t}`);
}
for (const [name, text] of [['ci.yml', ci], ['release.yml', release]]) {
  if (/\t/.test(text)) fail(`${name} contains a tab; YAML forbids tab indentation`);
  if (!/^on:/m.test(text) || !/^jobs:/m.test(text)) fail(`${name} lacks on: or jobs:`);
  if (!/cargo (test|nextest)/.test(ci) && name === 'ci.yml') fail('ci.yml runs no tests');
}
if (!/tags:/.test(release)) fail('release.yml is not triggered by tags');
if (!/makeappx|MakeAppx|msix/i.test(release)) fail('release.yml does not build an MSIX');
if (!/gh release|softprops\/action-gh-release|upload-release-asset/.test(release)) {
  fail('release.yml does not publish release assets');
}

const manifestPath = 'packaging/windows/AppxManifest.xml';
const manifest = read(manifestPath);
const stripped = manifest.replace(/<!--[\s\S]*?-->/g, '').replace(/<\?[\s\S]*?\?>/g, '');
const stack = [];
for (const m of stripped.matchAll(/<(\/?)([A-Za-z0-9:_.-]+)([^<>]*?)(\/?)>/g)) {
  const [, close, tag, , selfClose] = m;
  if (selfClose) continue;
  if (close) {
    if (stack.pop() !== tag) fail(`manifest XML is unbalanced at </${tag}>`);
  } else stack.push(tag);
}
if (stack.length) fail(`manifest XML leaves <${stack.pop()}> open`);
if (!/<Identity\b[^>]*\bName=/.test(manifest)) fail('manifest has no Identity Name');
if (/internetClient|privateNetworkClientServer|internetClientServer/.test(manifest)) {
  fail('manifest declares a network capability; the app must work offline');
}

const extPath = 'packaging/extensions.txt';
const wanted = read(extPath).split(/\r?\n/).map((s) => s.trim().toLowerCase()).filter(Boolean);
if (wanted.length < 20) fail(`extensions.txt lists only ${wanted.length} extensions`);
const associated = new Set(
  [...manifest.matchAll(/<uap:FileType[^>]*>\s*\.([A-Za-z0-9]+)\s*<\/uap:FileType>/g)].map((m) => m[1].toLowerCase()),
);
const unassociated = wanted.filter((e) => !associated.has(e));
if (unassociated.length) fail(`manifest lacks file associations for: ${unassociated.join(', ')}`);

const logos = [...manifest.matchAll(/(?:Logo|Square\d+x\d+Logo|Wide\d+x\d+Logo)="([^"]+)"|<Logo>([^<]+)<\/Logo>/g)]
  .map((m) => (m[1] || m[2]).replace(/\\/g, '/'));
if (logos.length < 3) fail('manifest references fewer than 3 logo assets');
for (const logo of new Set(logos)) {
  const p = join(dirname(manifestPath), logo);
  if (!existsSync(p)) fail(`logo asset missing: ${p}`);
  const sig = readFileSync(p).subarray(0, 8).toString('hex');
  if (sig !== '89504e470d0a1a0a') fail(`logo asset is not a PNG: ${p}`);
}

const env = { ...process.env };
env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
delete env.Path;
env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(process.cwd(), 'target-pandite-codecs');
const res = spawnSync('cargo', ['run', '-q', '-p', 'pandite-codecs', '--example', 'extensions'], {
  env, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
});
if (res.error || res.status !== 0) fail(`cannot list codec extensions\n${res.stderr}`);
const actual = res.stdout.split(/\r?\n/).map((s) => s.trim().toLowerCase()).filter(Boolean).sort();
const expected = [...wanted].sort();
if (actual.join(',') !== expected.join(',')) {
  const missing = actual.filter((e) => !expected.includes(e));
  const extra = expected.filter((e) => !actual.includes(e));
  fail(`extensions.txt differs from the codecs crate. missing: [${missing}] extra: [${extra}]`);
}

console.log(`${TARGETS.length} targets, ${wanted.length} associated extensions, ${new Set(logos).size} logos`);
console.log('PACKAGING VERIFIED');
