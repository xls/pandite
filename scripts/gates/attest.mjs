#!/usr/bin/env node
// Record reviewed evidence on a manual gate.
// Usage: node scripts/gates/attest.mjs <ledger> <gate-id> <evidence text>
// Refuses runnable gates (those are decided by gate-check only).
import { readFileSync, writeFileSync } from 'node:fs';
const [ledger, id, ...rest] = process.argv.slice(2);
const evidence = rest.join(' ').trim();
if (!ledger || !id || !evidence) { console.error('usage: attest.mjs <ledger> <gate-id> <evidence>'); process.exit(2); }
const lines = readFileSync(ledger, 'utf8').split('\n');
const start = lines.findIndex((l) => l.startsWith(`- [ ] ${id}:`) || l.startsWith(`- [x] ${id}:`));
if (start < 0) { console.error(`gate ${id} not found`); process.exit(1); }
let end = start + 1;
while (end < lines.length && /^\s+\S/.test(lines[end])) end += 1;
const body = lines.slice(start + 1, end);
if (body.some((l) => /^\s+(CHECK|EXPECT):/.test(l))) { console.error(`gate ${id} is runnable; refusing`); process.exit(1); }
lines[start] = lines[start].replace('- [ ]', '- [x]');
const kept = body.filter((l) => !/^\s+EVIDENCE:/.test(l));
lines.splice(start + 1, end - start - 1, ...kept, `  EVIDENCE: ${evidence}`);
writeFileSync(ledger, lines.join('\n'));
console.log(`attested ${id}`);
