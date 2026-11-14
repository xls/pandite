// The release file of each system, the systems without one, and the
// per-user folder of the installer.

import path from 'node:path';
import process from 'node:process';
import { InstallerError } from './errors.js';

export const REPO = 'xls/pandite';
export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASES_URL = `${REPO_URL}/releases`;
export const SUMS_FILE = 'SHA256SUMS.txt';
export const MIN_GLIBC = '2.35';
/** The AppId of the Windows setup program (packaging/windows/pandite.iss). */
export const WINDOWS_APP_ID = '{E5E712AA-D4BF-4DC9-BB19-9841DC38CD1E}';
export const WINDOWS_TARGETS = { x64: 'x86_64-pc-windows-msvc', arm64: 'aarch64-pc-windows-msvc' };
export const MAC_TARGET = 'universal-apple-darwin';
/** The app bundle at the top of the macOS zip and disk image. */
export const MAC_APP = 'Pandite.app';
export const LINUX_TARGET = 'x86_64-unknown-linux-gnu';
export const SUPPORTED =
  'Windows x64 and ARM64, macOS 11 or later (Apple Silicon and Intel), and Linux x64 with glibc 2.35 or later';

/** Every system with a Pandite release file, as Node.js names it. */
export const PLATFORMS = [
  { platform: 'win32', arch: 'x64' },
  { platform: 'win32', arch: 'arm64' },
  { platform: 'darwin', arch: 'x64' },
  { platform: 'darwin', arch: 'arm64' },
  { platform: 'linux', arch: 'x64' },
];

const OS_NAMES = { win32: 'Windows', darwin: 'macOS', linux: 'Linux' };

export function systemName(platform, arch) {
  return `${OS_NAMES[platform] ?? platform} ${arch}`;
}

function glibcBelow(have, want) {
  const a = String(have).split('.').map((n) => Number.parseInt(n, 10) || 0);
  const b = want.split('.').map(Number);
  for (let i = 0; i < b.length; i += 1) {
    const x = a[i] ?? 0;
    if (x !== b[i]) return x < b[i];
  }
  return false;
}

function unsupported(what) {
  return new InstallerError(
    `${what} is not supported. Pandite releases exist for ${SUPPORTED}. ` +
      `To build Pandite yourself, see ${REPO_URL}/blob/main/docs/building.md`,
  );
}

/** The folder at the top of the Linux tarball. */
export function tarballTop(version) {
  return `pandite-${version}-${LINUX_TARGET}`;
}

/**
 * The release files for a system, best first: [{ name, kind }]. Kinds:
 * "setup" (Windows setup program), "app-zip" and "dmg" (macOS app),
 * "tarball" (Linux). Throws an InstallerError for an unsupported system.
 */
export function candidateAssets({ platform, arch, glibc, version }) {
  const system = systemName(platform, arch);
  if (platform === 'win32') {
    const target = WINDOWS_TARGETS[arch];
    if (!target) throw unsupported(system);
    // The short name first; releases up to 0.2.2 carry the full target.
    return [
      { name: `pandite-${version}-${target.replace('-pc-windows-msvc', '-windows')}-setup.exe`, kind: 'setup' },
      { name: `pandite-${version}-${target}-setup.exe`, kind: 'setup' },
    ];
  }
  if (platform === 'darwin') {
    if (arch !== 'x64' && arch !== 'arm64') throw unsupported(system);
    const base = `pandite-${version}-${MAC_TARGET}`;
    return [
      { name: `${base}.zip`, kind: 'app-zip' },
      { name: `${base}.dmg`, kind: 'dmg' },
    ];
  }
  if (platform === 'linux' && arch === 'x64') {
    if (!glibc) throw unsupported('Linux without glibc (for example with musl)');
    if (glibcBelow(glibc, MIN_GLIBC)) throw unsupported(`Linux with glibc ${glibc}`);
    return [{ name: `${tarballTop(version)}.tar.gz`, kind: 'tarball' }];
  }
  throw unsupported(system);
}

/** Throws an InstallerError when no release file exists for this system. */
export function checkSupported(system) {
  candidateAssets({ ...system, version: '0.0.0' });
}

/**
 * The release file to download: the first candidate that `available` (the
 * file names of the release) holds.
 */
export function chooseAsset({ platform, arch, glibc, version, available }) {
  const candidates = candidateAssets({ platform, arch, glibc, version });
  const found = candidates.find((c) => available.includes(c.name));
  if (!found) {
    const names = candidates.map((c) => c.name).join(' or ');
    throw new InstallerError(`release v${version} has no ${names} for ${systemName(platform, arch)}`);
  }
  return found;
}

/** The glibc version of this Linux system, or null (musl, or not Linux). */
export function detectGlibc() {
  try {
    if (process.report && 'excludeNetwork' in process.report) process.report.excludeNetwork = true;
    return process.report?.getReport?.()?.header?.glibcVersionRuntime ?? null;
  } catch {
    return null;
  }
}

/** The per-user folder of the installer: the version marker and kept files. */
export function dataDir({ platform, env, home }) {
  if (platform === 'win32') {
    return path.join(env.LOCALAPPDATA || path.join(home, 'AppData', 'Local'), 'pandite-installer');
  }
  if (platform === 'darwin') return path.join(home, 'Library', 'Application Support', 'pandite-installer');
  const xdg = env.XDG_DATA_HOME && path.isAbsolute(env.XDG_DATA_HOME) ? env.XDG_DATA_HOME : path.join(home, '.local', 'share');
  return path.join(xdg, 'pandite-installer');
}
