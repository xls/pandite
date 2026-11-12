#!/usr/bin/env node
// Gate oracle: the shipped binary links no networking crate.
//
// Usage: node scripts/gates/no-network.mjs            (check the real tree)
//        node scripts/gates/no-network.mjs --control  (positive control)
//
// Reads the resolved normal + build dependency tree of the pandite binary
// for every release target and fails when a known networking crate is in
// it. The --control mode feeds the same matcher a tree that contains one,
// and passes only when the matcher flags it. Run both: an absence check is
// only trusted when its control fails as designed.

import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join, delimiter } from 'node:path';
import process from 'node:process';

const BANNED = [
  'reqwest', 'hyper', 'hyper-util', 'h2', 'h3', 'ureq', 'curl', 'curl-sys', 'isahc', 'attohttpc',
  'surf', 'minreq', 'tiny_http', 'tonic', 'quinn', 'rustls', 'native-tls', 'openssl', 'openssl-sys',
  'schannel', 'tokio-rustls', 'tokio-native-tls', 'tungstenite', 'websocket', 'socket2', 'mio',
  'trust-dns-resolver', 'hickory-resolver', 'dns-lookup', 'async-h1', 'http-client', 'sentry',
  'opentelemetry', 'self_update', 'octocrab',
];
const TARGETS = [
  'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'x86_64-apple-darwin',
  'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu',
];

function offenders(treeText) {
  const names = new Set();
  for (const line of treeText.split(/\r?\n/)) {
    const m = line.match(/^([A-Za-z0-9_-]+) v\d/);
    if (m) names.add(m[1]);
  }
  return BANNED.filter((b) => names.has(b));
}

if (process.argv.includes('--control')) {
  const fake = 'pandite-app v0.1.0\nimage v0.25.0\nreqwest v0.12.0\nmio v1.0.0\n';
  const hit = offenders(fake);
  if (hit.length === 2 && offenders('pandite-app v0.1.0\nimage v0.25.0\n').length === 0) {
    console.log('CONTROL DETECTED PLANTED NETWORK CRATES');
    process.exit(0);
  }
  console.error(`control failed: matcher returned ${JSON.stringify(hit)}`);
  process.exit(1);
}

const env = { ...process.env };
env.PATH = [join(homedir(), '.cargo', 'bin'), env.PATH ?? env.Path ?? ''].join(delimiter);
delete env.Path;

let total = 0;
for (const target of TARGETS) {
  const res = spawnSync(
    'cargo',
    ['tree', '-p', 'pandite-app', '--edges', 'normal,build', '--prefix', 'none', '--target', target],
    { env, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  );
  if (res.error || res.status !== 0) {
    console.error(`cargo tree failed for ${target}\n${res.stderr ?? res.error?.message}`);
    process.exit(1);
  }
  const lines = res.stdout.split(/\r?\n/).filter(Boolean).length;
  if (lines < 5) {
    console.error(`cargo tree for ${target} is implausibly small (${lines} lines)`);
    process.exit(1);
  }
  total += lines;
  const hit = offenders(res.stdout);
  if (hit.length > 0) {
    console.error(`networking crates in the ${target} tree: ${hit.join(', ')}`);
    process.exit(1);
  }
}
console.log(`${TARGETS.length} targets, ${total} dependency lines scanned`);
console.log('NO NETWORK CRATES LINKED');
