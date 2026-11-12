#!/usr/bin/env node
// Gate oracle for one workspace crate.
//
// Usage: node scripts/gates/crate.mjs <crate> [--tests a,b,c] [--features x,y] [--no-default-features]
//
// Runs, in order, for the named crate only:
//   1. cargo fmt --check
//   2. cargo clippy --all-targets with warnings denied
//   3. cargo test
// Then requires every name in --tests to appear as a PASSED test
// ("test <path containing name> ... ok"). A missing, failed or ignored
// required test fails the gate. Prints the success marker only after every
// step passed. Exits nonzero otherwise.

import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join, delimiter } from 'node:path';
import process from 'node:process';

const args = process.argv.slice(2);
const crate = args[0];
if (!crate || crate.startsWith('--')) {
  console.error('usage: crate.mjs <crate> [--tests a,b] [--features x,y] [--no-default-features]');
  process.exit(2);
}

function option(name) {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : '';
}
const required = option('--tests').split(',').map((s) => s.trim()).filter(Boolean);
const features = option('--features');
const noDefault = args.includes('--no-default-features');

const env = { ...process.env };
env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
delete env.Path;
// Each crate builds into its own directory so parallel leaves do not block
// on one build lock. Override with PANDITE_TARGET_DIR.
env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(process.cwd(), `target-${crate}`);
env.CARGO_TERM_COLOR = 'never';

// --env K=V (repeatable) sets variables for the cargo steps, for example
// PANDITE_REQUIRE_GPU=1 so a missing GPU adapter fails instead of skipping.
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === '--env' && args[i + 1] && args[i + 1].includes('=')) {
    const [key, ...rest] = args[i + 1].split('=');
    env[key] = rest.join('=');
  }
}

const featureArgs = [];
if (noDefault) featureArgs.push('--no-default-features');
if (features) featureArgs.push('--features', features);

function run(label, cargoArgs) {
  const res = spawnSync('cargo', cargoArgs, { env, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  const out = `${res.stdout ?? ''}\n${res.stderr ?? ''}`;
  if (res.error || res.status !== 0) {
    const tail = out.split(/\r?\n/).filter(Boolean).slice(-40).join('\n');
    console.error(`STEP FAILED: ${label}\n${res.error ? res.error.message : ''}\n${tail}`);
    process.exit(1);
  }
  return out;
}

run('fmt', ['fmt', '-p', crate, '--', '--check']);
run('clippy', ['clippy', '-p', crate, '--all-targets', ...featureArgs, '--', '-D', 'warnings']);
const testOut = run('test', ['test', '-p', crate, ...featureArgs]);

const passed = [...testOut.matchAll(/^test (\S+) \.\.\. ok\s*$/gm)].map((m) => m[1]);
if (passed.length === 0) {
  console.error('STEP FAILED: no test passed; a crate with zero tests cannot be verified');
  process.exit(1);
}
const missing = required.filter((name) => !passed.some((p) => p.includes(name)));
if (missing.length > 0) {
  console.error(`STEP FAILED: required tests did not pass: ${missing.join(', ')}`);
  process.exit(1);
}

console.log(`${passed.length} tests passed, ${required.length} required names present`);
console.log(`CRATE VERIFIED ${crate}`);
