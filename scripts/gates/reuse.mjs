// Result reuse for the gate oracles.
//
// Many ledgers name the same check. A check that passed on one exact source
// tree gives the same answer on that tree, so a second run only costs time.
// This module stores a passed result under target/gate-cache, keyed by the
// check, its arguments, its PANDITE_* environment and a digest of the source
// tree. A later run with the same key prints the stored output and a
// "REUSED" line, then exits 0.
//
// The tree digest covers the HEAD commit, every tracked change and every
// untracked file that git does not ignore. Any edit gives a new key.
// A stored result is used for 12 hours at most. Failed runs are never stored.
// The load-time benchmark is the exception: it is bound to the release
// binary and the sample files, has no time limit, and so runs again only
// when the binary changes.
// Set PANDITE_GATE_FRESH=1 to ignore stored results and run every check.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import process from 'node:process';

const MAX_AGE_MS = 12 * 60 * 60 * 1000;
const DIR = join(process.cwd(), 'target', 'gate-cache');

function git(args) {
  const res = spawnSync('git', args, { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  if (res.error || res.status !== 0) return null;
  return res.stdout;
}

let tree;
// Digest of the source tree, or '' when git cannot describe it (no reuse).
function treeDigest() {
  if (tree !== undefined) return tree;
  const head = git(['rev-parse', 'HEAD']);
  const diff = git(['diff', 'HEAD', '--binary']);
  const others = git(['ls-files', '--others', '--exclude-standard', '-z']);
  if (head === null || diff === null || others === null) {
    tree = '';
    return tree;
  }
  const hash = createHash('sha256').update(head).update('\0').update(diff).update('\0');
  for (const name of others.split('\0').filter(Boolean).sort()) {
    let stamp = 'missing';
    try {
      const st = statSync(name);
      stamp = `${st.size}:${st.mtimeMs}`;
    } catch { /* deleted meanwhile: the name alone changes the digest */ }
    hash.update(`${name}\0${stamp}\0`);
  }
  tree = hash.digest('hex');
  return tree;
}

// `bound` replaces the source tree digest for a check whose answer depends
// on something narrower, for example one built binary.
function keyFile(parts, bound) {
  const digest = bound ? JSON.stringify(bound) : treeDigest();
  if (!digest) return null;
  const env = Object.keys(process.env)
    .filter((k) => k.startsWith('PANDITE_') && k !== 'PANDITE_GATE_FRESH')
    .sort()
    .map((k) => `${k}=${process.env[k]}`);
  const key = createHash('sha256')
    .update(JSON.stringify([parts, env, digest, process.platform, process.arch]))
    .digest('hex');
  return join(DIR, `${key}.json`);
}

function load(file, maxAgeMs = MAX_AGE_MS) {
  if (!file || process.env.PANDITE_GATE_FRESH === '1') return null;
  try {
    const entry = JSON.parse(readFileSync(file, 'utf8'));
    if (typeof entry.output !== 'string' || typeof entry.at !== 'number') return null;
    if (Date.now() - entry.at > maxAgeMs || entry.at > Date.now()) return null;
    return entry;
  } catch {
    return null;
  }
}

function save(file, output) {
  if (!file) return;
  try {
    mkdirSync(DIR, { recursive: true });
    writeFileSync(file, JSON.stringify({ at: Date.now(), output }));
  } catch { /* no cache is not a gate failure */ }
}

// Whole-script reuse. Call first in an oracle. On a stored pass it prints
// that output and exits 0. Otherwise it records what the script prints to
// stdout and stores it when the script exits 0.
//
// Options: `bound` (array) keys the result to those values and not to the
// source tree; `maxAgeMs` changes the 12 hour limit (Infinity: no limit);
// `what` names the bound thing in the REUSED line.
export function reuseGate(name, args = process.argv.slice(2), options = {}) {
  const file = keyFile(['gate', name, args], options.bound);
  const hit = load(file, options.maxAgeMs);
  if (hit) {
    process.stdout.write(hit.output);
    console.log(`REUSED: ${name} passed on ${options.what ?? 'this exact source tree'} at ${new Date(hit.at).toISOString()}`);
    process.exit(0);
  }
  let seen = '';
  const write = process.stdout.write.bind(process.stdout);
  process.stdout.write = (chunk, ...rest) => {
    seen += typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8');
    return write(chunk, ...rest);
  };
  process.on('exit', (code) => {
    if (code === 0) save(file, seen);
  });
}

// Reuse of one expensive step inside an oracle. `produce` returns the text
// to keep and must end the process itself when the step fails.
export function reuseStep(parts, produce) {
  const file = keyFile(['step', parts]);
  const hit = load(file);
  if (hit) return { output: hit.output, reusedAt: new Date(hit.at).toISOString() };
  const output = produce();
  save(file, output);
  return { output, reusedAt: null };
}
