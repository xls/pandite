#!/usr/bin/env node
// Print the release version and check it against the tag.
//
// Usage: node packaging/version.mjs [--tag <vX.Y.Z>]
//
// The version comes from [workspace.package] in Cargo.toml. On a tag build
// (--tag, or GITHUB_REF_TYPE=tag with GITHUB_REF_NAME) the tag must be
// "v" + that version, or the script fails. Prints key=value lines and
// appends them to $GITHUB_OUTPUT when it is set:
//   version       0.1.0 or 0.2.0-rc.1
//   msix_version  0.1.0.0 (the Store reserves the fourth part)
//   prerelease    true when the version has a pre-release suffix
//   tag           the tag, or empty on a branch build

import { appendFileSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function fail(message) {
  console.error(`STEP FAILED: ${message}`);
  process.exit(1);
}

const toml = readFileSync(join(ROOT, 'Cargo.toml'), 'utf8');
const section = (toml.split(/^\[workspace\.package\]\s*$/m)[1] ?? '').split(/^\[/m)[0];
const version = section.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version) fail('cannot read [workspace.package] version from Cargo.toml');
const m = /^(\d+)\.(\d+)\.(\d+)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.exec(version);
if (!m) fail(`Cargo.toml version "${version}" is not semantic x.y.z`);
if (m.slice(1, 4).some((p) => Number(p) > 65535)) fail(`version ${version} has a part above 65535 (MSIX limit)`);

const args = process.argv.slice(2);
const i = args.indexOf('--tag');
let tag = i >= 0 ? (args[i + 1] ?? '') : '';
if (!tag && process.env.GITHUB_REF_TYPE === 'tag') tag = process.env.GITHUB_REF_NAME ?? '';
if (tag && tag !== `v${version}`) {
  fail(`tag ${tag} does not match the Cargo.toml version ${version}; tag v${version} or bump the version`);
}

const out = {
  version,
  msix_version: `${m[1]}.${m[2]}.${m[3]}.0`,
  prerelease: m[4] ? 'true' : 'false',
  tag,
};
const lines = Object.entries(out).map(([k, v]) => `${k}=${v}`);
console.log(lines.join('\n'));
if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join('\n')}\n`);
