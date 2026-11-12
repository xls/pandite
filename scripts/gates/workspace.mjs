#!/usr/bin/env node
// Gate oracle: the whole workspace composes.
//
// Usage: node scripts/gates/workspace.mjs
//
// Runs cargo fmt --check, clippy and test across every crate in one build,
// so interface mismatches between sibling crates surface here.

import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join, delimiter } from 'node:path';
import process from 'node:process';

const env = { ...process.env };
env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
delete env.Path;
env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(process.cwd(), 'target');
env.CARGO_TERM_COLOR = 'never';
env.PANDITE_REQUIRE_GPU = '1';

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

run('fmt', ['fmt', '--all', '--', '--check']);
run('clippy', ['clippy', '--workspace', '--all-targets', '--', '-D', 'warnings']);
const out = run('test', ['test', '--workspace']);
const passed = [...out.matchAll(/^test (\S+) \.\.\. ok\s*$/gm)].length;
const failed = [...out.matchAll(/^test (\S+) \.\.\. FAILED\s*$/gm)].length;
if (failed > 0 || passed < 50) {
  console.error(`STEP FAILED: ${passed} passed, ${failed} failed (expected at least 50 passing)`);
  process.exit(1);
}
console.log(`${passed} workspace tests passed`);
console.log('WORKSPACE VERIFIED');
