// Locate Windows SDK tools (makeappx.exe, makepri.exe, signtool.exe)
// without the registry.
//
// Search order for a tool:
//   1. the override variable (PANDITE_MAKEAPPX, PANDITE_MAKEPRI,
//      PANDITE_SIGNTOOL): full path to the .exe
//   2. %WindowsSdkVerBinPath%\<arch>\ (set by a Visual Studio developer prompt)
//   3. <Windows Kits 10>\bin\<newest 10.x version>\<arch>\ under
//      %ProgramFiles(x86)% and %ProgramFiles%
//   4. <Windows Kits 10>\bin\<arch>\ (old flat layout)
//   5. the directories on PATH

import { existsSync, readdirSync } from 'node:fs';
import { join, delimiter } from 'node:path';
import process from 'node:process';

const OVERRIDES = {
  'makeappx.exe': 'PANDITE_MAKEAPPX',
  'makepri.exe': 'PANDITE_MAKEPRI',
  'signtool.exe': 'PANDITE_SIGNTOOL',
};

function hostArchs() {
  // Prefer native binaries; x64 tools also run on ARM64 Windows.
  if (process.arch === 'arm64') return ['arm64', 'x64', 'x86'];
  if (process.arch === 'ia32') return ['x86'];
  return ['x64', 'x86'];
}

function versionKey(name) {
  return name.split('.').map((p) => p.padStart(6, '0')).join('.');
}

function kitRoots() {
  const roots = [
    process.env['ProgramFiles(x86)'],
    process.env.ProgramFiles,
    'C:\\Program Files (x86)',
    'C:\\Program Files',
  ]
    .filter(Boolean)
    .map((p) => join(p, 'Windows Kits', '10', 'bin'));
  return [...new Set(roots)].filter((p) => existsSync(p));
}

/** Every candidate path for `tool`, best first, without checking existence. */
export function candidates(tool) {
  const out = [];
  const override = OVERRIDES[tool] && process.env[OVERRIDES[tool]];
  if (override) out.push(override);
  const archs = hostArchs();
  if (process.env.WindowsSdkVerBinPath) {
    for (const a of archs) out.push(join(process.env.WindowsSdkVerBinPath, a, tool));
  }
  for (const root of kitRoots()) {
    let versions = [];
    try {
      versions = readdirSync(root).filter((d) => /^10\.\d+\.\d+\.\d+$/.test(d));
    } catch {
      versions = [];
    }
    versions.sort((a, b) => (versionKey(a) < versionKey(b) ? 1 : -1));
    for (const v of versions) for (const a of archs) out.push(join(root, v, a, tool));
    for (const a of archs) out.push(join(root, a, tool));
  }
  for (const dir of (process.env.PATH ?? process.env.Path ?? '').split(delimiter)) {
    if (dir) out.push(join(dir, tool));
  }
  return out;
}

/**
 * The path of a Windows SDK tool, or null when it is not installed.
 * An override variable that names a missing file is an error.
 */
export function findSdkTool(tool) {
  const name = OVERRIDES[tool];
  if (name && process.env[name] && !existsSync(process.env[name])) {
    throw new Error(`${name} is set to ${process.env[name]}, but that file does not exist`);
  }
  return candidates(tool).find((p) => existsSync(p)) ?? null;
}

/** A plain-language message for a missing SDK tool. */
export function missingToolMessage(tool) {
  return [
    `${tool} was not found.`,
    'Install the Windows SDK (Windows 10 SDK or later). In the Visual Studio Installer, select',
    '"Windows 11 SDK (10.0.26100.0)" or the standalone SDK from',
    'https://developer.microsoft.com/windows/downloads/windows-sdk/.',
    `To use a copy in another folder, set ${OVERRIDES[tool]} to the full path of ${tool}.`,
  ].join('\n');
}
