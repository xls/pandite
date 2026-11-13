// The command line of the Pandite installer: parse the arguments, build the
// context from the real system (or from the test doubles in `deps`) and
// run one command. main() resolves the exit code.

import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import os from 'node:os';
import process from 'node:process';
import { InstallerError, usageError } from './errors.js';
import { installCommand, listCommand, uninstallCommand, updateCommand } from './install.js';
import { checkSupported, detectGlibc } from './platform.js';
import { normalizeVersion } from './version.js';

export const INSTALLER_VERSION = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;

export const HELP = `Pandite installer ${INSTALLER_VERSION}
Installs Pandite, the fast offline image viewer, from its GitHub releases.

Usage:
  npx pandite [install] [-v <version>] [--force] [--system] [--dry-run]
  npx pandite update [--force] [--dry-run]
  npx pandite uninstall [--dry-run]
  npx pandite --list

Commands:
  install                 install the latest release, or the release of -v
                          (the default command)
  update                  install the latest release when it is newer than
                          the installed one; install it when Pandite is not
                          installed
  uninstall, --uninstall  remove Pandite

Options:
  -v, --version <version> the Pandite version to install: 0.1.0 or v0.1.0
  --list                  print the Pandite versions on GitHub
  --dry-run               print the downloads and the steps; download and
                          change nothing
  --force                 install again when the same version is installed
  --system                macOS: install in /Applications, not ~/Applications
  --installer-version     print the version of this installer
  -h, --help              print this help

pandite-installer is the same command.`;

const COMMANDS = new Set(['install', 'update', 'uninstall']);
const FLAGS = {
  '--list': 'list',
  '--dry-run': 'dryRun',
  '--force': 'force',
  '--system': 'system',
  '--installer-version': 'installerVersion',
  '-h': 'help',
  '--help': 'help',
};

/** The options of a command line. Throws a usage error (exit code 2). */
export function parseArgs(argv) {
  const opts = {
    command: null,
    version: null,
    list: false,
    dryRun: false,
    force: false,
    system: false,
    installerVersion: false,
    help: false,
  };
  const setCommand = (command) => {
    if (opts.command && opts.command !== command) throw usageError(`choose one command, not ${opts.command} and ${command}`);
    opts.command = command;
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = String(argv[i]);
    const eq = arg.startsWith('-') ? arg.indexOf('=') : -1;
    const name = eq > 0 ? arg.slice(0, eq) : arg;
    if (name === '-v' || name === '--version') {
      let value = eq > 0 ? arg.slice(eq + 1) : argv[i + 1];
      if (eq < 0) i += 1;
      if (value === undefined || value === '' || String(value).startsWith('-')) {
        throw usageError(`${name} needs a Pandite version, such as ${name} 0.1.0 (for the version of the installer: --installer-version)`);
      }
      value = normalizeVersion(value);
      if (opts.version && opts.version !== value) throw usageError(`choose one version, not ${opts.version} and ${value}`);
      opts.version = value;
    } else if (FLAGS[name]) {
      if (eq > 0) throw usageError(`${name} takes no value`);
      opts[FLAGS[name]] = true;
    } else if (name === '--uninstall') {
      if (eq > 0) throw usageError(`${name} takes no value`);
      setCommand('uninstall');
    } else if (arg.startsWith('-')) {
      throw usageError(`unknown option ${arg}`);
    } else if (COMMANDS.has(arg)) {
      setCommand(arg);
    } else {
      throw usageError(`unknown command "${arg}". This is the Pandite installer; it opens no images`);
    }
  }
  if (opts.version && (opts.command === 'update' || opts.command === 'uninstall')) {
    throw usageError(`${opts.command} takes no version. To install a named version, run: npx pandite install -v ${opts.version}`);
  }
  return opts;
}

/** Start a program and wait for it: { code, signal, stdout, stderr }. */
export function runProcess(command, args, { env, capture = false } = {}) {
  return new Promise((resolve, reject) => {
    const io = capture ? 'pipe' : 'ignore';
    const child = spawn(command, args, { env, stdio: ['ignore', io, io], windowsHide: true });
    const out = [];
    const err = [];
    child.stdout?.on('data', (chunk) => out.push(chunk));
    child.stderr?.on('data', (chunk) => err.push(chunk));
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolve({ code, signal, stdout: Buffer.concat(out).toString('utf8'), stderr: Buffer.concat(err).toString('utf8') });
    });
  });
}

/** The real system. Tests replace every key. */
export function defaultDeps() {
  return {
    platform: process.platform,
    arch: process.arch,
    glibc: undefined,
    env: process.env,
    home: os.homedir(),
    tmpdir: os.tmpdir(),
    systemApplications: '/Applications',
    fetch: globalThis.fetch,
    run: runProcess,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    now: () => Date.now(),
    out: (line) => process.stdout.write(`${line}\n`),
    err: (line) => process.stderr.write(`${line}\n`),
  };
}

function makeContext(deps) {
  const d = { ...defaultDeps(), ...deps };
  const glibc = d.glibc !== undefined ? d.glibc : d.platform === 'linux' ? detectGlibc() : null;
  return {
    platform: d.platform,
    arch: d.arch,
    glibc,
    env: d.env,
    home: d.home,
    tmpdir: d.tmpdir,
    systemApplications: d.systemApplications,
    run: d.run,
    sleep: d.sleep,
    net: { fetch: d.fetch, userAgent: `pandite-installer/${INSTALLER_VERSION}`, now: d.now },
    out: d.out,
    info: (line) => d.err(`pandite-installer: ${line}`),
    error: (line) => d.err(`pandite-installer: ${line}`),
  };
}

/** Run the installer with `argv`. Resolves the exit code: 0, 1, or 2 for a usage error. */
export async function main(argv, deps = {}) {
  const ctx = makeContext(deps);
  try {
    const opts = parseArgs(argv);
    if (opts.help) {
      ctx.out(HELP);
      return 0;
    }
    if (opts.installerVersion) {
      ctx.out(INSTALLER_VERSION);
      return 0;
    }
    if (typeof ctx.net.fetch !== 'function') throw new InstallerError('this Node.js has no fetch; use Node.js 18 or later');
    if (opts.list) return await listCommand(ctx);
    checkSupported(ctx);
    if (opts.system && ctx.platform !== 'darwin') throw usageError('--system works on macOS only');
    if (opts.command === 'update') return await updateCommand(ctx, opts);
    if (opts.command === 'uninstall') return await uninstallCommand(ctx, opts);
    return await installCommand(ctx, opts);
  } catch (error) {
    if (error instanceof InstallerError) {
      ctx.error(error.message);
      return error.exitCode;
    }
    ctx.error(`unexpected error: ${error?.stack ?? error}`);
    return 1;
  }
}
