// Install, update and remove Pandite. Every access to the system and the
// network goes through the context `ctx`, so test.mjs runs all of it
// offline:
//   platform, arch, glibc  the system, as Node.js names it
//   env, home, tmpdir      environment, home folder, temporary folder
//   systemApplications     the macOS folder of --system (/Applications)
//   run(command, args, { env, capture })
//                          start a program; resolves { code, signal, stdout, stderr }
//   sleep(ms)              wait
//   net                    { fetch, userAgent, now } for net.js
//   out(line), info(line)  result lines (stdout) and progress lines (stderr)

import {
  appendFileSync,
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { untar, unzip } from './archive.js';
import { InstallerError } from './errors.js';
import { getRelease, listReleases, parseSums } from './github.js';
import { download, getText } from './net.js';
import {
  MAC_APP,
  RELEASES_URL,
  SUMS_FILE,
  WINDOWS_APP_ID,
  chooseAsset,
  dataDir,
  systemName,
  tarballTopOf,
} from './platform.js';
import { compareVersions } from './version.js';

/** Arguments of the Windows setup program (Inno Setup): silent, current user. */
export const SETUP_ARGS = ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/CURRENTUSER'];
export const UNINSTALL_ARGS = ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'];
/** The uninstall entry that the setup program writes, under HKCU or HKLM. */
export const UNINSTALL_KEY = `Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${WINDOWS_APP_ID}_is1`;
export const SH = '/bin/sh';
export const HDIUTIL = '/usr/bin/hdiutil';
export const MARKER = 'installed.json';
export const SETUP_RECORD = 'linux-setup.json';
export const XDG_MIME = 'xdg-mime';
export const DESKTOP_FILE = 'pandite.desktop';
export const DEFAULT_QUESTION = 'Make Pandite the default viewer for the image formats that it opens? [y/N] ';
export const FLOAT_QUESTION = 'Open Pandite as a floating window in Hyprland? [y/N] ';
export const HYPRLAND_RULE = 'o.window("pandite", { tag = "+floating-window" })';
const REGISTRY_ROOTS = ['HKCU', 'HKLM'];
const UNINSTALL_WAIT_MS = 60_000;
const UNINSTALL_POLL_MS = 1_000;

// ------------------------------------------------------------------ helpers

export function regExe(env) {
  return path.join(env.SystemRoot || env.SYSTEMROOT || 'C:\\Windows', 'System32', 'reg.exe');
}

export function markerFile(ctx) {
  return path.join(dataDir(ctx), MARKER);
}

export function setupLog(ctx) {
  return path.join(dataDir(ctx), 'setup.log');
}

function linuxPrefix(ctx) {
  return path.join(ctx.home, '.local');
}

function linuxKeep(ctx) {
  return path.join(dataDir(ctx), 'linux');
}

function macAppsDir(ctx, system) {
  return system ? ctx.systemApplications : path.join(ctx.home, 'Applications');
}

function windowsDefaultDir(ctx) {
  return path.join(ctx.env.LOCALAPPDATA || path.join(ctx.home, 'AppData', 'Local'), 'Programs', 'Pandite');
}

function megabytes(bytes) {
  return bytes ? ` (${(bytes / 1024 / 1024).toFixed(1)} MB)` : '';
}

function lastLine(text) {
  return String(text ?? '').trim().split(/\r?\n/).pop() ?? '';
}

function exitText(r) {
  return r.code === null || r.code === undefined ? `signal ${r.signal}` : `exit code ${r.code}`;
}

function cleanup(dir) {
  try {
    rmSync(dir, { recursive: true, force: true, maxRetries: 3 });
  } catch {
    // A temporary folder that stays behind does no harm.
  }
}

async function run(ctx, command, args, options = {}) {
  try {
    return await ctx.run(command, args, options);
  } catch (error) {
    throw new InstallerError(`cannot start ${command}: ${error?.message ?? error}`);
  }
}

/** -1, 0 or 1; an installed version that is not semantic counts as older. */
function compareInstalled(installed, version) {
  try {
    return compareVersions(installed, version);
  } catch {
    return -1;
  }
}

// ------------------------------------------------------------------- marker

/** The marker of the installed version: { version, ... }, or null. */
export function readMarker(ctx) {
  try {
    const marker = JSON.parse(readFileSync(markerFile(ctx), 'utf8'));
    return marker && typeof marker.version === 'string' ? marker : null;
  } catch {
    return null;
  }
}

function writeMarker(ctx, marker) {
  mkdirSync(dataDir(ctx), { recursive: true });
  writeFileSync(markerFile(ctx), `${JSON.stringify(marker, null, 2)}\n`);
}

/** Delete the per-user folder of the installer: marker, log, kept files. */
function forget(ctx) {
  rmSync(dataDir(ctx), { recursive: true, force: true });
}

// ----------------------------------------------------------------- Windows

/** The values of a .reg file that `reg export` writes (UTF-16 or UTF-8). */
export function parseRegFile(buf) {
  const text = buf[0] === 0xff && buf[1] === 0xfe ? buf.toString('utf16le', 2) : buf.toString('utf8');
  const values = {};
  for (const line of text.split(/\r?\n/)) {
    const m = /^"((?:[^"\\]|\\.)*)"="((?:[^"\\]|\\.)*)"$/.exec(line);
    if (m) values[m[1].replace(/\\(.)/g, '$1')] = m[2].replace(/\\(.)/g, '$1');
  }
  return values;
}

function commandPath(command) {
  const text = String(command ?? '').trim();
  const quoted = /^"([^"]+)"/.exec(text);
  return quoted ? quoted[1] : text.split(/\s+\//)[0];
}

/**
 * The uninstall entry of the setup program: { version, location,
 * uninstaller }, or null. `reg export` reads it without administrator
 * rights and writes Unicode, so paths with any characters survive.
 */
async function windowsEntry(ctx) {
  const dir = mkdtempSync(path.join(ctx.tmpdir, 'pandite-installer-'));
  try {
    for (const root of REGISTRY_ROOTS) {
      const file = path.join(dir, `${root}.reg`);
      const r = await run(ctx, regExe(ctx.env), ['export', `${root}\\${UNINSTALL_KEY}`, file, '/y'], { capture: true });
      if (r.code !== 0 || !existsSync(file)) continue;
      const values = parseRegFile(readFileSync(file));
      const uninstaller = commandPath(values.UninstallString) || null;
      const location = (values.InstallLocation || (uninstaller ? path.dirname(uninstaller) : '')).replace(/[\\/]+$/, '');
      if (!uninstaller && !location) continue;
      return { version: values.DisplayVersion || null, location, uninstaller };
    }
    return null;
  } finally {
    cleanup(dir);
  }
}

// ------------------------------------------------------------ installed

/**
 * The installed Pandite: { version, location, ... }, or null. On Windows
 * the uninstall entry of the setup program decides; elsewhere the marker
 * and the installed program decide.
 */
export async function installedState(ctx) {
  const marker = readMarker(ctx);
  if (ctx.platform === 'win32') {
    const entry = await windowsEntry(ctx);
    return entry ? { ...entry, version: entry.version ?? marker?.version ?? null } : null;
  }
  if (!marker) return null;
  if (ctx.platform === 'darwin') {
    if (!marker.location || !existsSync(marker.location)) return null;
    return { version: marker.version, location: marker.location, system: Boolean(marker.system) };
  }
  const prefix = marker.prefix || linuxPrefix(ctx);
  const bin = path.join(prefix, 'bin', 'pandite');
  return existsSync(bin) ? { version: marker.version, location: bin, prefix } : null;
}

// ------------------------------------------------------------ installers

async function installSetup(ctx, { file, release }) {
  mkdirSync(dataDir(ctx), { recursive: true });
  const log = setupLog(ctx);
  rmSync(log, { force: true });
  ctx.info('running the setup program for the current user');
  const r = await run(ctx, file, [...SETUP_ARGS, `/LOG=${log}`]);
  if (r.code !== 0) throw new InstallerError(`the setup program failed with ${exitText(r)}. Its log is ${log}`);
  const entry = await windowsEntry(ctx);
  const location = entry?.location || windowsDefaultDir(ctx);
  return {
    marker: { location },
    message: `Pandite ${release.version} installed in ${location}. Start it from the Start menu.`,
  };
}

function macStage(dir, system) {
  try {
    mkdirSync(dir, { recursive: true });
    return mkdtempSync(path.join(dir, '.pandite-installer-'));
  } catch (error) {
    const hint = system ? ' Use an administrator account, or install without --system.' : '';
    throw new InstallerError(`cannot write to ${dir}: ${error?.code ?? error?.message}.${hint}`);
  }
}

/** Move `stage`/Pandite.app to `dir`, in place of an older copy. */
function placeApp(stage, dir, release, system) {
  const app = path.join(stage, MAC_APP);
  const exe = path.join(app, 'Contents', 'MacOS', 'pandite');
  if (!existsSync(exe)) throw new InstallerError(`the release file holds no ${MAC_APP}/Contents/MacOS/pandite`);
  chmodSync(exe, 0o755);
  const dest = path.join(dir, MAC_APP);
  const old = path.join(stage, `old-${MAC_APP}`);
  if (existsSync(dest)) renameSync(dest, old);
  try {
    renameSync(app, dest);
  } catch (error) {
    if (existsSync(old)) renameSync(old, dest);
    throw new InstallerError(`cannot move ${MAC_APP} to ${dir}: ${error?.message ?? error}`);
  }
  return {
    marker: { location: dest, system },
    message: `Pandite ${release.version} installed in ${dest}. Start it from Launchpad or with: open "${dest}"`,
  };
}

async function installAppZip(ctx, { file, release, system }) {
  const dir = macAppsDir(ctx, system);
  const stage = macStage(dir, system);
  try {
    unzip(readFileSync(file), stage);
    return placeApp(stage, dir, release, system);
  } finally {
    cleanup(stage);
  }
}

async function installDmg(ctx, { file, work, release, system }) {
  const mount = path.join(work, 'mount');
  mkdirSync(mount);
  const attach = await run(ctx, HDIUTIL, ['attach', '-nobrowse', '-readonly', '-noautoopen', '-mountpoint', mount, file], {
    capture: true,
  });
  if (attach.code !== 0) {
    throw new InstallerError(`hdiutil cannot mount ${path.basename(file)}: ${lastLine(attach.stderr) || exitText(attach)}`);
  }
  try {
    if (!existsSync(path.join(mount, MAC_APP))) throw new InstallerError(`${path.basename(file)} holds no ${MAC_APP}`);
    const dir = macAppsDir(ctx, system);
    const stage = macStage(dir, system);
    try {
      cpSync(path.join(mount, MAC_APP), path.join(stage, MAC_APP), { recursive: true, verbatimSymlinks: true });
      return placeApp(stage, dir, release, system);
    } finally {
      cleanup(stage);
    }
  } finally {
    await run(ctx, HDIUTIL, ['detach', mount, '-force'], { capture: true }).catch(() => {});
  }
}

async function installTarball(ctx, { file, work, release }) {
  const unpacked = path.join(work, 'unpacked');
  mkdirSync(unpacked);
  untar(readFileSync(file), unpacked);
  const top = path.join(unpacked, tarballTopOf(path.basename(file)));
  const script = path.join(top, 'install.sh');
  if (!existsSync(script) || !existsSync(path.join(top, 'pandite'))) {
    throw new InstallerError(`${path.basename(file)} holds no ${tarballTopOf(path.basename(file))}/install.sh and pandite`);
  }
  const prefix = linuxPrefix(ctx);
  ctx.info(`running install.sh with PREFIX=${prefix}`);
  const r = await run(ctx, SH, [script], { env: { ...ctx.env, PREFIX: prefix }, capture: true });
  if (r.code !== 0) {
    throw new InstallerError(`install.sh failed with ${exitText(r)}: ${lastLine(r.stderr) || lastLine(r.stdout)}`);
  }
  // install.sh --uninstall needs the script and the icon folders beside it.
  const keep = linuxKeep(ctx);
  rmSync(keep, { recursive: true, force: true });
  mkdirSync(keep, { recursive: true });
  cpSync(script, path.join(keep, 'install.sh'));
  chmodSync(path.join(keep, 'install.sh'), 0o755);
  if (existsSync(path.join(top, 'share'))) cpSync(path.join(top, 'share'), path.join(keep, 'share'), { recursive: true });
  const bin = path.join(prefix, 'bin', 'pandite');
  return {
    marker: { prefix, location: bin },
    message: `Pandite ${release.version} installed in ${bin}. Start it from the application menu or with: ${bin}`,
  };
}

/** The MIME types of the desktop entry that the last install kept. */
function desktopMimeTypes(ctx) {
  const file = path.join(linuxKeep(ctx), 'share', 'applications', DESKTOP_FILE);
  let text;
  try {
    text = readFileSync(file, 'utf8');
  } catch {
    return [];
  }
  const line = /^MimeType=(.*)$/m.exec(text);
  return line
    ? line[1]
        .split(';')
        .map((t) => t.trim())
        .filter(Boolean)
    : [];
}

/** What the Linux setup changed, kept outside the install marker: it stays through updates. */
function setupFile(ctx) {
  return path.join(dataDir(ctx), SETUP_RECORD);
}

function readSetup(ctx) {
  try {
    const record = JSON.parse(readFileSync(setupFile(ctx), 'utf8'));
    return record && typeof record === 'object' ? record : {};
  } catch {
    return {};
  }
}

function writeSetup(ctx, record) {
  try {
    mkdirSync(dataDir(ctx), { recursive: true });
    writeFileSync(setupFile(ctx), `${JSON.stringify(record, null, 2)}\n`);
  } catch {
    // Without the record, the uninstall leaves the defaults as they are.
  }
}

/** The default program of a MIME type, or '' when none is set or xdg-mime fails. */
async function defaultOf(ctx, type) {
  try {
    const r = await ctx.run(XDG_MIME, ['query', 'default', type], { env: ctx.env, capture: true });
    return r.code === 0 ? r.stdout.trim() : '';
  } catch {
    return '';
  }
}

/** Linux: make Pandite the default program of its MIME types. Never fails the install. */
async function setDefaultViewer(ctx) {
  const types = desktopMimeTypes(ctx);
  if (types.length === 0) {
    ctx.info('the desktop entry of Pandite lists no MIME types; the default viewer was not changed');
    return false;
  }
  // Remember the program each type had before, for the uninstall. A type
  // that is on record keeps its first answer.
  const record = readSetup(ctx);
  const previous = record.previous && typeof record.previous === 'object' ? record.previous : {};
  for (const type of types) {
    if (type in previous) continue;
    const program = await defaultOf(ctx, type);
    if (program !== DESKTOP_FILE) previous[type] = program;
  }
  let r;
  try {
    r = await ctx.run(XDG_MIME, ['default', DESKTOP_FILE, ...types], { env: ctx.env, capture: true });
  } catch (error) {
    ctx.info(`cannot start ${XDG_MIME} (${error?.message ?? error}); the default viewer was not changed. Install xdg-utils and run: npx pandite --default`);
    return false;
  }
  if (r.code !== 0) {
    ctx.info(`${XDG_MIME} failed with ${exitText(r)}: ${lastLine(r.stderr) || lastLine(r.stdout)}; the default viewer was not changed`);
    return false;
  }
  writeSetup(ctx, { ...readSetup(ctx), previous });
  ctx.out(`Pandite is the default viewer for ${types.length} image formats.`);
  return true;
}

/** True when xdg-mime is a program in PATH: the system uses the XDG defaults. */
function hasXdgMime(ctx) {
  return String(ctx.env.PATH ?? '')
    .split(path.delimiter)
    .some((dir) => dir !== '' && existsSync(path.join(dir, XDG_MIME)));
}

/** The Hyprland configuration that takes the floating window rule. */
export function hyprlandFile(ctx) {
  return path.join(ctx.home, '.config', 'hypr', 'hyprland.lua');
}

/** Linux with Hyprland: add the floating window rule once. Never fails the install. */
function setFloating(ctx) {
  const file = hyprlandFile(ctx);
  let text;
  try {
    text = readFileSync(file, 'utf8');
  } catch {
    ctx.info(`${file} was not found; the floating window rule was not added`);
    return false;
  }
  if (/^\s*o\.window\(\s*["']pandite["']/m.test(text)) {
    ctx.out(`${file} has a window rule for Pandite already; it was not changed.`);
    return true;
  }
  try {
    appendFileSync(file, `${text === '' || text.endsWith('\n') ? '' : '\n'}${HYPRLAND_RULE}\n`);
  } catch (error) {
    ctx.info(`cannot write ${file}: ${error?.message ?? error}; the floating window rule was not added`);
    return false;
  }
  writeSetup(ctx, { ...readSetup(ctx), hyprland: true });
  ctx.out(`Added the floating window rule for Pandite to ${file}.`);
  return true;
}

const yes = (answer) => /^y(es)?$/i.test(String(answer ?? '').trim());

/**
 * Linux, after an install: do what --default and --float name, ask on a
 * first install when someone can answer, and ask nothing with --no-default
 * and --no-float. The default viewer question comes only when the system
 * has xdg-mime, the floating window question only when the Hyprland
 * configuration exists.
 */
async function offerLinuxSetup(ctx, opts, fresh) {
  if (ctx.platform !== 'linux') return;
  const canAsk = fresh && Boolean(ctx.ask);
  if (opts.makeDefault) await setDefaultViewer(ctx);
  else if (!opts.noDefault && fresh && hasXdgMime(ctx)) {
    if (!ctx.ask) ctx.info('to make Pandite the default image viewer, run: npx pandite --default');
    else if (yes(await ctx.ask(DEFAULT_QUESTION))) await setDefaultViewer(ctx);
  }
  if (opts.float) setFloating(ctx);
  else if (!opts.noFloat && canAsk && existsSync(hyprlandFile(ctx))) {
    if (yes(await ctx.ask(FLOAT_QUESTION))) setFloating(ctx);
  }
}

/** The lines of --dry-run for the Linux setup. */
function linuxSetupPlan(ctx, opts, fresh) {
  if (ctx.platform !== 'linux') return [];
  const lines = [];
  const canAsk = fresh && Boolean(ctx.ask);
  if (opts.makeDefault) lines.push(`Would run ${XDG_MIME} default ${DESKTOP_FILE} with the MIME types of the desktop entry`);
  else if (!opts.noDefault && canAsk && hasXdgMime(ctx)) lines.push('Would ask whether Pandite becomes the default image viewer');
  if (opts.float) lines.push(`Would add the floating window rule for Pandite to ${hyprlandFile(ctx)}`);
  else if (!opts.noFloat && canAsk && existsSync(hyprlandFile(ctx))) {
    lines.push('Would ask whether Pandite opens as a floating window in Hyprland');
  }
  return lines;
}

/** --default or --float with nothing to install: set up the installed Pandite. */
async function setupOfInstalled(ctx, opts) {
  if (ctx.platform !== 'linux' || !(opts.makeDefault || opts.float)) return;
  if (opts.dryRun) {
    for (const line of linuxSetupPlan(ctx, opts, false)) ctx.out(line);
    return;
  }
  await offerLinuxSetup(ctx, opts, false);
}

const INSTALL = { setup: installSetup, 'app-zip': installAppZip, dmg: installDmg, tarball: installTarball };

/** The steps of each kind of release file, for --dry-run. */
function plan(ctx, kind, { file, temp, release, system }) {
  if (kind === 'setup') {
    return [
      `Would run: ${file} ${[...SETUP_ARGS, `/LOG=${setupLog(ctx)}`].join(' ')}`,
      `Would install in ${windowsDefaultDir(ctx)} (or the folder of an earlier install), for the current user, without administrator rights`,
    ];
  }
  const dest = path.join(macAppsDir(ctx, system), MAC_APP);
  if (kind === 'app-zip') return [`Would unpack ${MAC_APP} from ${path.basename(file)} and move it to ${dest}, in place of an older copy`];
  if (kind === 'dmg') return [`Would mount ${path.basename(file)} with ${HDIUTIL} and copy ${MAC_APP} to ${dest}, in place of an older copy`];
  const script = path.join(temp, 'unpacked', tarballTopOf(path.basename(file)), 'install.sh');
  return [
    `Would unpack ${path.basename(file)} and run: PREFIX=${linuxPrefix(ctx)} ${SH} ${script}`,
    `Would keep a copy of install.sh in ${linuxKeep(ctx)} for the uninstall`,
  ];
}

// ---------------------------------------------------------------- commands

/** Download, check and install `release`. */
async function deliver(ctx, release, opts, installed) {
  const available = release.assets.map((a) => a.name);
  const pick = chooseAsset({ platform: ctx.platform, arch: ctx.arch, glibc: ctx.glibc, version: release.version, available });
  const asset = release.assets.find((a) => a.name === pick.name);
  const sums = release.assets.find((a) => a.name === SUMS_FILE);
  if (!sums) {
    throw new InstallerError(`release ${release.tag} has no ${SUMS_FILE}, so its files cannot be checked. Nothing was installed.`);
  }
  const system = ctx.platform === 'darwin' && Boolean(opts.system || installed?.system);
  const label = `Pandite ${release.version}${release.latest ? ' (latest release)' : ''} for ${systemName(ctx.platform, ctx.arch)}`;

  if (opts.dryRun) {
    const temp = path.join(ctx.tmpdir, 'pandite-installer-XXXXXX');
    const file = path.join(temp, pick.name);
    ctx.out(label);
    ctx.out(`Would download ${asset.url}${megabytes(asset.size)}`);
    ctx.out(`Would download ${sums.url} and check the SHA-256 of ${pick.name} against it`);
    for (const line of plan(ctx, pick.kind, { file, temp, release, system })) ctx.out(line);
    ctx.out(`Would write the version to ${markerFile(ctx)}`);
    for (const line of linuxSetupPlan(ctx, opts, !installed)) ctx.out(line);
    ctx.out('Dry run: nothing was downloaded, installed or changed.');
    return 0;
  }

  ctx.info(label);
  const work = mkdtempSync(path.join(ctx.tmpdir, 'pandite-installer-'));
  try {
    const got = await getText(sums.url, ctx.net, { accept: 'application/octet-stream' });
    if (got.status !== 200) throw new InstallerError(`download of ${sums.url} failed: HTTP ${got.status}`);
    const expected = parseSums(got.text).get(pick.name);
    if (!expected) {
      throw new InstallerError(`${SUMS_FILE} of release ${release.tag} does not list ${pick.name}. Nothing was installed.`);
    }
    ctx.info(`downloading ${asset.url}${megabytes(asset.size)}`);
    const file = path.join(work, pick.name);
    const { sha256 } = await download(asset.url, file, ctx.net);
    if (sha256 !== expected) {
      rmSync(file, { force: true });
      throw new InstallerError(
        `SHA-256 mismatch for ${pick.name}: ${SUMS_FILE} lists ${expected}, the download has ${sha256}. ` +
          'The download was deleted. Nothing was installed.',
      );
    }
    ctx.info(`the SHA-256 of ${pick.name} matches ${SUMS_FILE}`);
    const done = await INSTALL[pick.kind](ctx, { file, work, release, system });
    writeMarker(ctx, { version: release.version, asset: pick.name, sha256, ...done.marker });
    ctx.out(done.message);
    await offerLinuxSetup(ctx, opts, !installed);
    return 0;
  } finally {
    cleanup(work);
  }
}

function alreadyInstalled(installed) {
  return `Pandite ${installed.version} is already installed in ${installed.location}. To install it again, add --force.`;
}

/** `install`: the latest release, or `opts.version`. */
export async function installCommand(ctx, opts) {
  const installed = await installedState(ctx);
  if (opts.version && installed && !opts.force && compareInstalled(installed.version, opts.version) === 0) {
    ctx.out(alreadyInstalled(installed));
    await setupOfInstalled(ctx, opts);
    return 0;
  }
  const release = await getRelease(opts.version, ctx.net);
  if (!opts.version && installed && !opts.force) {
    const order = compareInstalled(installed.version, release.version);
    if (order === 0) {
      ctx.out(alreadyInstalled(installed));
      await setupOfInstalled(ctx, opts);
      return 0;
    }
    if (order > 0) {
      ctx.out(
        `Pandite ${installed.version} is installed in ${installed.location}. It is newer than the latest release ` +
          `${release.version}. To install ${release.version}, run: npx pandite install -v ${release.version}`,
      );
      return 0;
    }
  }
  return deliver(ctx, release, opts, installed);
}

/** `update`: the latest release, when it is newer than the installed one. */
export async function updateCommand(ctx, opts) {
  const installed = await installedState(ctx);
  const release = await getRelease(null, ctx.net);
  if (installed && !opts.force && compareInstalled(installed.version, release.version) >= 0) {
    ctx.out(`Pandite ${installed.version} is already up to date (latest release: ${release.version}).`);
    await setupOfInstalled(ctx, opts);
    return 0;
  }
  if (installed) ctx.info(`updating Pandite ${installed.version ?? '(unknown version)'} to ${release.version}`);
  else ctx.info(`Pandite is not installed; installing the latest release ${release.version}`);
  return deliver(ctx, release, opts, installed);
}

function notInstalled(ctx, opts, sentence) {
  if (!opts.dryRun) forget(ctx);
  ctx.out(`${sentence} Nothing was removed.`);
  return 0;
}

function dryRunEnd(ctx) {
  ctx.out(`Would delete ${dataDir(ctx)}`);
  ctx.out('Dry run: nothing was changed.');
  return 0;
}

async function uninstallWindows(ctx, opts) {
  const entry = await windowsEntry(ctx);
  if (!entry) return notInstalled(ctx, opts, 'Pandite is not installed for this user: the setup program registered no uninstaller.');
  if (!entry.uninstaller || !existsSync(entry.uninstaller)) {
    throw new InstallerError(
      `the registered uninstaller ${entry.uninstaller ?? '(none)'} does not exist. Remove Pandite in Settings, Apps, Installed apps.`,
    );
  }
  if (opts.dryRun) {
    ctx.out(`Would run: ${entry.uninstaller} ${UNINSTALL_ARGS.join(' ')}`);
    return dryRunEnd(ctx);
  }
  ctx.info('running the uninstaller');
  const r = await run(ctx, entry.uninstaller, UNINSTALL_ARGS);
  if (r.code !== 0) throw new InstallerError(`the uninstaller failed with ${exitText(r)}`);
  // The uninstaller finishes in a copy of itself: wait for its entry to go.
  for (let waited = 0; await windowsEntry(ctx); waited += UNINSTALL_POLL_MS) {
    if (waited >= UNINSTALL_WAIT_MS) {
      throw new InstallerError(`the uninstaller did not finish within ${UNINSTALL_WAIT_MS / 1000} seconds`);
    }
    await ctx.sleep(UNINSTALL_POLL_MS);
  }
  forget(ctx);
  ctx.out(`Pandite removed from ${entry.location}.`);
  return 0;
}

async function uninstallMac(ctx, opts) {
  const marker = readMarker(ctx);
  const location = marker?.location || path.join(macAppsDir(ctx, opts.system), MAC_APP);
  if (path.basename(location) !== MAC_APP || !existsSync(location)) {
    return notInstalled(ctx, opts, `Pandite is not installed: ${location} does not exist.`);
  }
  if (opts.dryRun) {
    ctx.out(`Would delete ${location}`);
    return dryRunEnd(ctx);
  }
  try {
    rmSync(location, { recursive: true, force: true });
  } catch (error) {
    throw new InstallerError(`cannot delete ${location}: ${error?.code ?? error?.message}`);
  }
  forget(ctx);
  ctx.out(`Pandite removed from ${location}.`);
  return 0;
}

/**
 * Linux, at uninstall: give every MIME type that still opens with Pandite
 * back to the program it had before, and take the floating window rule out
 * of the Hyprland configuration. Never fails the uninstall.
 */
async function undoLinuxSetup(ctx) {
  const record = readSetup(ctx);
  const previous = record.previous && typeof record.previous === 'object' ? record.previous : {};
  const back = new Map();
  for (const [type, program] of Object.entries(previous)) {
    if (typeof program !== 'string' || program === '' || program === DESKTOP_FILE) continue;
    // A type that the user gave to another program since stays there.
    if ((await defaultOf(ctx, type)) !== DESKTOP_FILE) continue;
    back.set(program, [...(back.get(program) ?? []), type]);
  }
  let restored = 0;
  for (const [program, types] of back) {
    try {
      const r = await ctx.run(XDG_MIME, ['default', program, ...types], { env: ctx.env, capture: true });
      if (r.code === 0) restored += types.length;
    } catch {
      // xdg-mime is gone: nothing to restore with.
    }
  }
  if (restored > 0) ctx.out(`The default viewer of ${restored} image formats is the program from before Pandite again.`);
  if (record.hyprland) {
    const file = hyprlandFile(ctx);
    try {
      const text = readFileSync(file, 'utf8');
      const kept = text.split('\n').filter((line) => line.trim() !== HYPRLAND_RULE);
      if (kept.length !== text.split('\n').length) {
        writeFileSync(file, kept.join('\n'));
        ctx.out(`Removed the floating window rule for Pandite from ${file}.`);
      }
    } catch {
      // The file is gone or cannot be written: leave it.
    }
  }
}

async function uninstallLinux(ctx, opts) {
  const marker = readMarker(ctx);
  const script = path.join(linuxKeep(ctx), 'install.sh');
  const prefix = marker?.prefix || linuxPrefix(ctx);
  if (!existsSync(script)) {
    return notInstalled(ctx, opts, `Pandite is not installed by this installer: ${linuxKeep(ctx)} holds no install.sh.`);
  }
  if (opts.dryRun) {
    ctx.out(`Would run: PREFIX=${prefix} ${SH} ${script} --uninstall`);
    const record = readSetup(ctx);
    if (record.previous && Object.keys(record.previous).length > 0) {
      ctx.out('Would give the image formats that open with Pandite back to the program from before');
    }
    if (record.hyprland) ctx.out(`Would remove the floating window rule for Pandite from ${hyprlandFile(ctx)}`);
    return dryRunEnd(ctx);
  }
  const r = await run(ctx, SH, [script, '--uninstall'], { env: { ...ctx.env, PREFIX: prefix }, capture: true });
  if (r.code !== 0) {
    throw new InstallerError(`install.sh --uninstall failed with ${exitText(r)}: ${lastLine(r.stderr) || lastLine(r.stdout)}`);
  }
  await undoLinuxSetup(ctx);
  forget(ctx);
  ctx.out(`Pandite removed from ${prefix}.`);
  return 0;
}

/** `uninstall`: remove Pandite and the marker. */
export async function uninstallCommand(ctx, opts) {
  if (ctx.platform === 'win32') return uninstallWindows(ctx, opts);
  if (ctx.platform === 'darwin') return uninstallMac(ctx, opts);
  return uninstallLinux(ctx, opts);
}

/** `--list`: the release versions, newest first. */
export async function listCommand(ctx) {
  const releases = await listReleases(ctx.net);
  if (!releases.length) throw new InstallerError(`no Pandite release found at ${RELEASES_URL}`);
  for (const r of releases) ctx.out(r.prerelease ? `${r.version} (pre-release)` : r.version);
  return 0;
}
