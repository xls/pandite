#!/usr/bin/env node
// The Pandite installer: npx pandite [install|update|uninstall] [options]
// Run with --help for the commands and options.

import process from 'node:process';

const major = Number.parseInt(process.versions.node, 10);
if (major < 18) {
  process.stderr.write(`pandite-installer: Node.js 18 or later is required; this is Node.js ${process.versions.node}\n`);
  process.exit(1);
}

const { main } = await import('../lib/cli.js');
process.exitCode = await main(process.argv.slice(2));
