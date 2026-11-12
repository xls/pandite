#!/usr/bin/env node
// Gate oracle: every numbered requirement of a RAW specification is cited
// by the decoder source or its tests, so no requirement was skipped silently.
//
// Usage: node scripts/gates/rawtrace.mjs <format>   (for example: arw)
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import process from 'node:process';

const fmt = process.argv[2];
if (!fmt) { console.error('usage: rawtrace.mjs <format>'); process.exit(2); }
const specPath = `specs/raw/${fmt}.md`;
if (!existsSync(specPath)) { console.error(`STEP FAILED: ${specPath} missing`); process.exit(1); }
const tag = `R-${fmt.toUpperCase()}-`;
const wanted = new Set([...readFileSync(specPath, 'utf8').matchAll(new RegExp(`\b${tag}\d+\b`, 'g'))].map((m) => m[0]));
if (wanted.size < 25) { console.error(`STEP FAILED: spec has only ${wanted.size} requirements`); process.exit(1); }
const found = new Set();
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (p.endsWith('.rs')) {
      for (const m of readFileSync(p, 'utf8').matchAll(new RegExp(`\b${tag}\d+\b`, 'g'))) found.add(m[0]);
    }
  }
};
walk('crates/pandite-codecs/src');
walk('crates/pandite-codecs/tests');
const missing = [...wanted].filter((r) => !found.has(r)).sort((a, b) => a.localeCompare(b, 'en', { numeric: true }));
if (missing.length) { console.error(`STEP FAILED: ${missing.length} requirements not cited in code or tests: ${missing.join(', ')}`); process.exit(1); }
console.log(`${wanted.size} requirements, all cited`);
console.log('RAW REQUIREMENTS TRACED');
