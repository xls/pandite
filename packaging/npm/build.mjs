#!/usr/bin/env node
// Build the npm launcher package of a release.
//
// Usage: node packaging/npm/build.mjs --version <x.y.z> --sums <file> --out <dir>
//
// <file> lists the SHA-256 of the release packages in sha256sum format.
// The script copies the launcher package to a temporary folder, writes
// checksums.json with the SHA-256 of every release file the launcher
// downloads, runs npm pack there and writes <dir>/pandite-<version>-npm.tgz.
// The version must equal "version" in packaging/npm/package.json.

import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { allAssets } from './lib/launcher.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');

/** Parse sha256sum output: Map of file name to lowercase SHA-256. */
export function parseSums(text) {
  const sums = new Map();
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue;
    const m = /^([0-9a-fA-F]{64}) [ *](.+)$/.exec(line);
    if (!m) throw new Error(`not a sha256sum line: ${line}`);
    sums.set(m[2], m[1].toLowerCase());
  }
  return sums;
}

/** checksums.json: the SHA-256 of every file the launcher downloads. */
export function makeChecksums(version, sums) {
  const sha256 = {};
  for (const asset of allAssets(version)) {
    const hash = sums.get(asset);
    if (!hash) throw new Error(`the sums file has no line for ${asset}`);
    sha256[asset] = hash;
  }
  return { version, sha256 };
}

function copyTree(from, to) {
  mkdirSync(to, { recursive: true });
  for (const name of readdirSync(from)) {
    const source = path.join(from, name);
    if (statSync(source).isDirectory()) copyTree(source, path.join(to, name));
    else copyFileSync(source, path.join(to, name));
  }
}

function npm(args, cwd) {
  const options = { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] };
  // npm is a .cmd file on Windows, which runs in a shell: one quoted command line.
  const result =
    process.platform === 'win32'
      ? spawnSync(['npm.cmd', ...args.map((a) => `"${a}"`)].join(' '), { ...options, shell: true })
      : spawnSync('npm', args, options);
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`npm ${args.join(' ')} failed with exit code ${result.status}`);
  return result.stdout;
}

export function build({ version, sumsFile, outDir }) {
  const pkg = JSON.parse(readFileSync(path.join(HERE, 'package.json'), 'utf8'));
  if (pkg.version !== version) {
    throw new Error(`packaging/npm/package.json has version ${pkg.version}, the release is ${version}`);
  }
  const checksums = makeChecksums(version, parseSums(readFileSync(sumsFile, 'utf8')));
  const out = path.resolve(outDir);
  mkdirSync(out, { recursive: true });
  const stage = mkdtempSync(path.join(os.tmpdir(), 'pandite-npm-'));
  try {
    copyFileSync(path.join(HERE, 'package.json'), path.join(stage, 'package.json'));
    copyFileSync(path.join(HERE, 'README.md'), path.join(stage, 'README.md'));
    copyFileSync(path.join(ROOT, 'LICENSE'), path.join(stage, 'LICENSE'));
    copyTree(path.join(HERE, 'bin'), path.join(stage, 'bin'));
    copyTree(path.join(HERE, 'lib'), path.join(stage, 'lib'));
    writeFileSync(path.join(stage, 'checksums.json'), `${JSON.stringify(checksums, null, 2)}\n`);
    const [packed] = JSON.parse(npm(['pack', '--json', '--ignore-scripts', '--pack-destination', out], stage));
    const listed = packed.files.map((f) => f.path).sort();
    for (const need of ['LICENSE', 'README.md', 'bin/pandite.js', 'checksums.json', 'lib/launcher.js', 'package.json']) {
      if (!listed.includes(need)) throw new Error(`npm pack left out ${need}`);
    }
    const target = path.join(out, `pandite-${version}-npm.tgz`);
    rmSync(target, { force: true });
    renameSync(path.join(out, packed.filename), target);
    console.log(listed.join('\n'));
    console.log(JSON.stringify(checksums, null, 2));
    console.log(`wrote ${target}`);
    return target;
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

function option(args, name) {
  const i = args.indexOf(name);
  const value = i >= 0 ? args[i + 1] : undefined;
  if (!value) throw new Error(`missing ${name}; usage: node packaging/npm/build.mjs --version <x.y.z> --sums <file> --out <dir>`);
  return value;
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  try {
    const args = process.argv.slice(2);
    build({ version: option(args, '--version'), sumsFile: option(args, '--sums'), outDir: option(args, '--out') });
  } catch (error) {
    console.error(`STEP FAILED: ${error.message}`);
    process.exit(1);
  }
}
