#!/usr/bin/env node
// Gate oracle: a RAW format specification document is complete enough to
// hand to an implementer who has no other knowledge of the format.
//
// Usage: node scripts/gates/rawspec.mjs <format> [<format>...]
//
// For each specs/raw/<format>.md, requires every mandatory section with real
// content, a sources table whose every row names a public URL and a source
// kind, numbered normative requirements, and at least one worked numeric
// example. Also requires specs/raw/PROVENANCE.md. Structure only: whether
// the content is correct is proven later by the decoder's pixel tests.
import { readFileSync, existsSync } from 'node:fs';
import process from 'node:process';

const fail = (m) => { console.error(`STEP FAILED: ${m}`); process.exit(1); };
const formats = process.argv.slice(2);
if (formats.length === 0) fail('no format named');
const SECTIONS = [
  'Sources', 'Scope and variants', 'Container layout', 'Metadata tags', 'Sensor data layout',
  'Compression', 'Levels and white balance', 'Color conversion', 'Decoding procedure',
  'Embedded preview', 'Error handling', 'Test vectors', 'Open questions',
];
if (!existsSync('specs/raw/PROVENANCE.md')) fail('specs/raw/PROVENANCE.md is missing');
const prov = readFileSync('specs/raw/PROVENANCE.md', 'utf8');
if (prov.length < 400) fail('PROVENANCE.md is too short to state the method');
let total = 0;
for (const f of formats) {
  const path = `specs/raw/${f}.md`;
  if (!existsSync(path)) fail(`${path} is missing`);
  const text = readFileSync(path, 'utf8');
  if (/[^\x00-\x7F]/.test(text)) fail(`${path} contains non-ASCII characters`);
  const parts = text.split(/^## +/m);
  const byTitle = new Map(parts.slice(1).map((p) => [p.split('\n')[0].trim(), p.split('\n').slice(1).join('\n')]));
  for (const s of SECTIONS) {
    const body = byTitle.get(s);
    if (body === undefined) fail(`${path} lacks section "## ${s}"`);
    if (body.replace(/\s+/g, ' ').trim().length < 200) fail(`${path} section "${s}" has under 200 characters`);
  }
  const rows = byTitle.get('Sources').split('\n').filter((l) => /^\|\s*S\d+\s*\|/.test(l));
  if (rows.length < 4) fail(`${path} lists ${rows.length} sources (S1, S2, ... rows); at least 4 needed`);
  for (const r of rows) {
    if (!/https?:\/\/\S+/.test(r)) fail(`${path} source row has no URL: ${r.slice(0, 80)}`);
    if (!/\b(official|standard|patent|vendor|community documentation|measurement)\b/i.test(r)) {
      fail(`${path} source row has no kind (official, standard, patent, vendor, community documentation, measurement): ${r.slice(0, 80)}`);
    }
  }
  const reqs = new Set([...text.matchAll(/\bR-[A-Z0-9]+-\d+\b/g)].map((m) => m[0]));
  if (reqs.size < 25) fail(`${path} has ${reqs.size} numbered requirements (R-XXX-n); at least 25 needed`);
  if (!/0x[0-9A-Fa-f]{2,}/.test(byTitle.get('Test vectors'))) fail(`${path} test vectors contain no hex bytes`);
  total += reqs.size;
  console.log(`${path}: ${rows.length} sources, ${reqs.size} requirements`);
}
console.log(`${formats.length} specification(s), ${total} requirements`);
console.log('RAW SPEC STRUCTURE VERIFIED');
