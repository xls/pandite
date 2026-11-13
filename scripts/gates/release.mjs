#!/usr/bin/env node
// Gate oracle: the release binary builds, stays under the size budget and
// passes its own headless smoke test.
//
// Usage: node scripts/gates/release.mjs [--budget-mb N] [--smoke <image>]...
//
// 1. cargo build --release -p pandite-app
// 2. measure the binary on disk and compare with the budget
// 3. for each --smoke image: run `pandite --smoke <image>`; it must exit 0
//    and print "SMOKE OK" (the app opens the file, renders frames through
//    the real GPU path, and exits without user input)

import { spawnSync } from 'node:child_process';
import { statSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, delimiter } from 'node:path';
import process from 'node:process';
import { reuseGate } from './reuse.mjs';

reuseGate('release');

const args = process.argv.slice(2);
let budgetMb = 16;
const smokes = [];
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === '--budget-mb') budgetMb = Number(args[i + 1]);
  if (args[i] === '--smoke') smokes.push(args[i + 1]);
}
if (!Number.isFinite(budgetMb) || budgetMb <= 0) {
  console.error('bad --budget-mb');
  process.exit(2);
}

const env = { ...process.env };
env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
delete env.Path;
env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(process.cwd(), 'target');
env.CARGO_TERM_COLOR = 'never';

const build = spawnSync('cargo', ['build', '--release', '-p', 'pandite-app'], {
  env, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024,
});
if (build.error || build.status !== 0) {
  const tail = `${build.stdout}\n${build.stderr}`.split(/\r?\n/).filter(Boolean).slice(-40).join('\n');
  console.error(`STEP FAILED: release build\n${tail}`);
  process.exit(1);
}

const exe = join(env.CARGO_TARGET_DIR, 'release', process.platform === 'win32' ? 'pandite.exe' : 'pandite');
if (!existsSync(exe)) {
  console.error(`STEP FAILED: binary not found at ${exe}`);
  process.exit(1);
}
const bytes = statSync(exe).size;
const mb = bytes / (1024 * 1024);
console.log(`binary size: ${bytes} bytes (${mb.toFixed(2)} MiB), budget ${budgetMb} MiB`);
if (mb > budgetMb) {
  console.error('STEP FAILED: binary exceeds the size budget');
  process.exit(1);
}

for (const image of smokes) {
  if (!existsSync(image)) {
    console.error(`STEP FAILED: smoke fixture missing: ${image}`);
    process.exit(1);
  }
  const run = spawnSync(exe, ['--smoke', image], { env, encoding: 'utf8', timeout: 120000 });
  const out = `${run.stdout ?? ''}\n${run.stderr ?? ''}`;
  if (run.error || run.status !== 0 || !out.includes('SMOKE OK')) {
    console.error(`STEP FAILED: smoke run for ${image} (exit ${run.status})\n${out.slice(-2000)}`);
    process.exit(1);
  }
  console.log(`smoke passed: ${image}`);
}
if (smokes.length === 0) {
  console.error('STEP FAILED: no --smoke fixture given; the release gate needs at least one');
  process.exit(1);
}
console.log('RELEASE VERIFIED');
