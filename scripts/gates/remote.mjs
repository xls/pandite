#!/usr/bin/env node
// Gate oracle: the published state matches local work.
//
// Usage: node scripts/gates/remote.mjs        (remote main == local HEAD, clean tree)
//        node scripts/gates/remote.mjs --ci   (CI run for HEAD succeeded)

import { spawnSync } from 'node:child_process';
import process from 'node:process';

function sh(cmd, args) {
  const res = spawnSync(cmd, args, { encoding: 'utf8' });
  if (res.error || res.status !== 0) {
    console.error(`STEP FAILED: ${cmd} ${args.join(' ')}\n${res.stderr ?? res.error?.message}`);
    process.exit(1);
  }
  return res.stdout.trim();
}

const head = sh('git', ['rev-parse', 'HEAD']);
if (!/^[0-9a-f]{40}$/.test(head)) {
  console.error('STEP FAILED: no HEAD commit');
  process.exit(1);
}

if (!process.argv.includes('--ci')) {
  if (sh('git', ['branch', '--show-current']) !== 'main') {
    console.error('STEP FAILED: current branch is not main');
    process.exit(1);
  }
  const dirty = sh('git', ['status', '--porcelain']);
  if (dirty) {
    console.error(`STEP FAILED: working tree is not clean\n${dirty.slice(0, 1000)}`);
    process.exit(1);
  }
  const remote = sh('git', ['ls-remote', 'origin', 'refs/heads/main']).split(/\s+/)[0];
  if (remote !== head) {
    console.error(`STEP FAILED: remote main is ${remote || 'absent'}, local head is ${head}`);
    process.exit(1);
  }
  console.log(`head ${head}`);
  console.log('REMOTE MAIN MATCHES LOCAL HEAD');
  process.exit(0);
}

const json = sh('gh', ['run', 'list', '--commit', head, '--workflow', 'ci.yml', '--json', 'databaseId,status,conclusion', '--limit', '5']);
const runs = JSON.parse(json);
if (runs.length === 0) {
  console.error(`STEP FAILED: no CI run found for ${head}`);
  process.exit(1);
}
const run = runs[0];
if (run.status !== 'completed' || run.conclusion !== 'success') {
  console.error(`STEP FAILED: CI run ${run.databaseId} is ${run.status}/${run.conclusion}`);
  process.exit(1);
}
const jobs = JSON.parse(sh('gh', ['run', 'view', String(run.databaseId), '--json', 'jobs'])).jobs;
const bad = jobs.filter((j) => j.conclusion !== 'success');
if (jobs.length < 4 || bad.length > 0) {
  console.error(`STEP FAILED: ${jobs.length} jobs, not successful: ${bad.map((j) => j.name).join(', ')}`);
  process.exit(1);
}
console.log(`run ${run.databaseId}: ${jobs.length} jobs succeeded`);
console.log('CI SUCCEEDED FOR HEAD');
