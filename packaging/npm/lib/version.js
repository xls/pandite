// Semantic versions (https://semver.org): parse, normalize and compare.

import { usageError } from './errors.js';

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

/** { major, minor, patch, pre: [identifiers] } of a version, or null. */
export function parseVersion(text) {
  const m = SEMVER.exec(String(text));
  if (!m) return null;
  return { major: m[1], minor: m[2], patch: m[3], pre: m[4] ? m[4].split('.') : [] };
}

/**
 * The version that the user typed after -v, without a leading "v".
 * "0.1.0" and "v0.1.0" give "0.1.0"; "1.0" gives "1.0.0" and "1" gives
 * "1.0.0". Throws a usage error for anything else.
 */
export function normalizeVersion(input) {
  let text = String(input ?? '').trim();
  if (/^[vV]/.test(text)) text = text.slice(1);
  const short = /^(0|[1-9]\d*)(?:\.(0|[1-9]\d*))?$/.exec(text);
  if (short) text = `${short[1]}.${short[2] ?? '0'}.0`;
  if (!parseVersion(text)) throw usageError(`"${input}" is not a version such as 0.1.0 or v0.1.0`);
  return text;
}

/** Compare two digit strings of any length as numbers. */
function compareDigits(a, b) {
  if (a.length !== b.length) return a.length < b.length ? -1 : 1;
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

function compareIdentifiers(a, b) {
  const an = /^\d+$/.test(a);
  const bn = /^\d+$/.test(b);
  if (an && bn) return compareDigits(a, b);
  // Numeric identifiers have lower precedence than alphanumeric ones.
  if (an) return -1;
  if (bn) return 1;
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

/**
 * -1 when a is older than b, 0 when they are equal, 1 when a is newer.
 * 0.10.0 is newer than 0.9.0, and 1.0.0-rc.1 is older than 1.0.0. Build
 * metadata (+...) does not count. Throws for an invalid version.
 */
export function compareVersions(a, b) {
  const x = parseVersion(a);
  const y = parseVersion(b);
  if (!x || !y) throw new Error(`cannot compare "${a}" with "${b}": not semantic versions`);
  for (const key of ['major', 'minor', 'patch']) {
    const order = compareDigits(x[key], y[key]);
    if (order) return order;
  }
  if (!x.pre.length || !y.pre.length) return x.pre.length === y.pre.length ? 0 : x.pre.length ? -1 : 1;
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i += 1) {
    const order = compareIdentifiers(x.pre[i], y.pre[i]);
    if (order) return order;
  }
  if (x.pre.length === y.pre.length) return 0;
  return x.pre.length < y.pre.length ? -1 : 1;
}
