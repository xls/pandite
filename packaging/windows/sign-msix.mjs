#!/usr/bin/env node
// Sign MSIX packages or bundles for sideloading. The Microsoft Store does
// not need this: it signs the packages it accepts.
//
// Usage:
//   node packaging/windows/sign-msix.mjs --pfx <cert.pfx> [--timestamp <url>] <file>...
//
// The certificate password comes from PANDITE_SIGN_PASSWORD (empty when
// unset). The certificate subject must equal the Publisher value the
// package was built with, or signtool rejects the package.

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import process from 'node:process';
import { findSdkTool, missingToolMessage } from './sdk-tools.mjs';

function fail(message) {
  console.error(`STEP FAILED: ${message}`);
  process.exit(1);
}

const args = process.argv.slice(2);
let pfx = process.env.PANDITE_SIGN_PFX || '';
let timestamp = '';
const files = [];
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === '--pfx') pfx = args[(i += 1)] ?? '';
  else if (args[i] === '--timestamp') timestamp = args[(i += 1)] ?? '';
  else if (args[i].startsWith('--')) fail(`unknown argument: ${args[i]}`);
  else files.push(resolve(args[i]));
}
if (!pfx) fail('no certificate: pass --pfx <file> or set PANDITE_SIGN_PFX');
if (!existsSync(pfx)) fail(`certificate not found: ${pfx}`);
if (files.length === 0) fail('no package to sign');

let signtool;
try {
  signtool = findSdkTool('signtool.exe');
} catch (err) {
  fail(err.message);
}
if (!signtool) fail(missingToolMessage('signtool.exe'));

for (const file of files) {
  if (!existsSync(file)) fail(`package not found: ${file}`);
  const signArgs = ['sign', '/fd', 'SHA256', '/f', pfx, '/p', process.env.PANDITE_SIGN_PASSWORD ?? ''];
  if (timestamp) signArgs.push('/tr', timestamp, '/td', 'SHA256');
  signArgs.push(file);
  const res = spawnSync(signtool, signArgs, { encoding: 'utf8' });
  if (res.error || res.status !== 0) {
    // Never echo the arguments: they hold the password.
    fail(`signtool could not sign ${file} (exit ${res.status})\n${res.stdout ?? ''}${res.stderr ?? ''}`);
  }
  console.log(`signed ${file}`);
}
console.log('MSIX SIGNED');
