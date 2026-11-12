#!/usr/bin/env node
// Gate oracle: large images reach the screen within the time budget.
//
// Usage: node scripts/gates/perf.mjs <sample-folder> [--runs N]
//
// For every .arw, .jpg and .hif file in the folder, runs the release binary
// `pandite --smoke <file> --timing` N times (default 3) and reads the line
//   TIMING first_pixel_ms=<n> full_res_ms=<n> ...
// that the app prints. Both figures count from the moment the app asks its
// library for the file (window already visible, GPU ready) to the presented
// frame that first shows the image at any resolution (first_pixel) and at
// full resolution (full_res). The median of the runs is compared with the
// budget for the file type and pixel count. The GPU line must name a
// discrete adapter. The sample folder is private and local: this gate can
// only run on the owner's machine.

import { spawnSync } from 'node:child_process';
import { readdirSync, existsSync } from 'node:fs';
import { join, extname } from 'node:path';
import process from 'node:process';

const fail = (m) => { console.error(`STEP FAILED: ${m}`); process.exit(1); };
const args = process.argv.slice(2);
const folder = args[0];
const runsAt = args.indexOf('--runs');
const runs = runsAt >= 0 ? Number(args[runsAt + 1]) : 3;
if (!folder || !existsSync(folder)) fail('sample folder missing');
if (!Number.isInteger(runs) || runs < 1) fail('bad --runs');

// Budgets in milliseconds for images of 40 megapixels and more. Smaller
// images get the same first-pixel budget and a full-resolution budget
// scaled by pixel count, never below 250 ms.
const BUDGET = {
  '.arw': { first: 200, full: 600 },
  '.jpg': { first: 300, full: 900 },
  '.hif': { first: 2500, full: 2500 }, // bound by the operating system codec
};
const REF_PIXELS = 60e6;

const exe = join(process.cwd(), 'target', 'release', process.platform === 'win32' ? 'pandite.exe' : 'pandite');
if (!existsSync(exe)) fail(`release binary missing at ${exe}; build it first`);
const files = readdirSync(folder).filter((f) => BUDGET[extname(f).toLowerCase()]).sort();
const kinds = new Set(files.map((f) => extname(f).toLowerCase()));
if (!kinds.has('.arw') || !kinds.has('.jpg')) fail('folder needs at least one .arw and one .jpg file');

const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
let worst = 0;
for (const f of files) {
  const ext = extname(f).toLowerCase();
  const first = [];
  const full = [];
  let pixels = 0;
  for (let i = 0; i < runs; i += 1) {
    const res = spawnSync(exe, ['--smoke', join(folder, f), '--timing'], { encoding: 'utf8', timeout: 120000 });
    const out = `${res.stdout ?? ''}\n${res.stderr ?? ''}`;
    if (res.error || res.status !== 0 || !out.includes('SMOKE OK')) fail(`smoke run failed for ${f}\n${out.slice(-600)}`);
    if (!/gpu [^\n]*discrete/.test(out)) fail(`run for ${f} did not use a discrete GPU`);
    const t = out.match(/TIMING first_pixel_ms=(\d+(?:\.\d+)?) full_res_ms=(\d+(?:\.\d+)?)/);
    if (!t) fail(`no TIMING line for ${f}`);
    const dim = out.match(/ (\d+)x(\d+) /);
    if (!dim) fail(`no dimensions in smoke output for ${f}`);
    pixels = Number(dim[1]) * Number(dim[2]);
    first.push(Number(t[1]));
    full.push(Number(t[2]));
  }
  const b = BUDGET[ext];
  const fullBudget = Math.max(250, Math.round(b.full * Math.min(1, pixels / REF_PIXELS)));
  const mFirst = median(first);
  const mFull = median(full);
  console.log(`${f}: ${(pixels / 1e6).toFixed(1)} MP first ${mFirst} ms (budget ${b.first}), full ${mFull} ms (budget ${fullBudget})`);
  if (mFull < mFirst) fail(`${f}: full_res before first_pixel is impossible`);
  if (mFirst > b.first) fail(`${f}: first pixel ${mFirst} ms exceeds ${b.first} ms`);
  if (mFull > fullBudget) fail(`${f}: full resolution ${mFull} ms exceeds ${fullBudget} ms`);
  worst = Math.max(worst, mFull);
}
console.log(`${files.length} files, ${runs} runs each, slowest full resolution ${worst} ms`);
console.log('PERF WITHIN BUDGET');
