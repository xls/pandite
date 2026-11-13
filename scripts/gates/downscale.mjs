#!/usr/bin/env node
// Gate oracle: the Rgba8 thumbnail downscaler is fast enough.
//
// Usage: node scripts/gates/downscale.mjs [--min-speedup N]
//
// Builds and runs the release benchmark example of pandite-library:
//   cargo run --release -p pandite-library --example downscale_bench -- --sizes 6000x4000
// It times the f64 reference (the previous implementation) and the fast
// path (box_downscale at the level this CPU selects) in one process on the
// same 6000x4000 Rgba8 input, best of several runs each, once opaque and
// once translucent, and prints for each
//   DOWNSCALE_SPEEDUP size=6000x4000 alpha=<a> level=<l> reference_ms=<f> fast_ms=<f> speedup=<f> max_diff=<n>
// Requires both lines, every speedup >= --min-speedup (default 8) and
// max_diff <= 1. The example itself exits nonzero when a vector level
// differs from the scalar path. Prints the success marker only after every
// check passed. Exits nonzero otherwise.

import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join, delimiter } from 'node:path';
import process from 'node:process';

const fail = (m) => { console.error(`STEP FAILED: ${m}`); process.exit(1); };
const args = process.argv.slice(2);
const at = args.indexOf('--min-speedup');
const minSpeedup = at >= 0 ? Number(args[at + 1]) : 8;
if (!Number.isFinite(minSpeedup) || minSpeedup <= 0) fail('bad --min-speedup');
const SIZE = '6000x4000';

const env = { ...process.env };
env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
delete env.Path;
// Shares the build directory of the crate gate (crate.mjs pandite-library).
// Override with PANDITE_TARGET_DIR.
env.CARGO_TARGET_DIR = env.PANDITE_TARGET_DIR || join(process.cwd(), 'target-pandite-library');
env.CARGO_TERM_COLOR = 'never';
// Measure the level this CPU selects, not a forced scalar run.
delete env.PANDITE_SIMD;

const res = spawnSync(
  'cargo',
  ['run', '--release', '-p', 'pandite-library', '--example', 'downscale_bench', '--', '--sizes', SIZE],
  { env, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
);
const out = `${res.stdout ?? ''}\n${res.stderr ?? ''}`;
if (res.error || res.status !== 0) {
  const tail = out.split(/\r?\n/).filter(Boolean).slice(-40).join('\n');
  fail(`benchmark run\n${res.error ? res.error.message : ''}\n${tail}`);
}
for (const line of out.split(/\r?\n/)) {
  if (/^(levels on this CPU|DOWNSCALE )/.test(line)) console.log(line);
}

const pattern = /^DOWNSCALE_SPEEDUP size=(\d+x\d+) alpha=(\S+) level=(\S+) reference_ms=([\d.]+) fast_ms=([\d.]+) speedup=([\d.]+) max_diff=(\d+)\s*$/gm;
const results = [...out.matchAll(pattern)].map((m) => ({
  size: m[1], alpha: m[2], level: m[3], reference: Number(m[4]), fast: Number(m[5]), maxDiff: Number(m[7]),
}));
for (const alpha of ['opaque', 'translucent']) {
  const r = results.find((x) => x.size === SIZE && x.alpha === alpha);
  if (!r) fail(`no DOWNSCALE_SPEEDUP line for ${SIZE} ${alpha}`);
  if (!(r.reference > 0 && r.fast > 0)) fail(`${alpha}: times must be positive`);
  const speedup = r.reference / r.fast;
  console.log(`${SIZE} ${alpha}: reference ${r.reference.toFixed(3)} ms, ${r.level} ${r.fast.toFixed(3)} ms, speedup ${speedup.toFixed(2)} (minimum ${minSpeedup}), max difference ${r.maxDiff}`);
  if (r.maxDiff > 1) fail(`${alpha}: result differs from the reference by ${r.maxDiff} steps; at most 1 allowed`);
  if (speedup < minSpeedup) fail(`${alpha}: speedup ${speedup.toFixed(2)} is below ${minSpeedup}`);
}
console.log('DOWNSCALE FAST ENOUGH');
