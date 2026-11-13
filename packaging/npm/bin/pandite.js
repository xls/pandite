#!/usr/bin/env node
// npx pandite [options] [--] [<image>]
// Runs Pandite. The first run downloads the release for this system.

import os from 'node:os';
import process from 'node:process';
import { LauncherError, main } from '../lib/launcher.js';

try {
  const { code, signal } = await main(process.argv.slice(2));
  process.exitCode = signal ? 128 + (os.constants.signals[signal] ?? 0) : (code ?? 1);
} catch (error) {
  process.stderr.write(`pandite: ${error instanceof LauncherError ? error.message : (error?.stack ?? String(error))}\n`);
  process.exitCode = 1;
}
