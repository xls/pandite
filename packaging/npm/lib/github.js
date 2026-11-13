// The Pandite releases on GitHub: the latest release, a named release, the
// list of versions, and the SHA256SUMS.txt format. Anonymous requests only.

import { InstallerError } from './errors.js';
import { getText } from './net.js';
import { RELEASES_URL, REPO } from './platform.js';
import { compareVersions, parseVersion } from './version.js';

export const API_URL = `https://api.github.com/repos/${REPO}`;
const API_HEADERS = { accept: 'application/vnd.github+json', 'x-github-api-version': '2022-11-28' };
const MAX_PAGES = 10;

function retryText(headers, now) {
  const after = Number.parseInt(headers.get('retry-after') ?? '', 10);
  const reset = Number.parseInt(headers.get('x-ratelimit-reset') ?? '', 10);
  let seconds = null;
  if (after >= 0) seconds = after;
  else if (reset > 0) seconds = reset - Math.floor(now() / 1000);
  if (seconds === null) return 'later';
  const minutes = Math.max(1, Math.ceil(seconds / 60));
  return `in ${minutes} minute${minutes === 1 ? '' : 's'}`;
}

/** GET an API address: { json, headers }, or null for HTTP 404. */
async function api(url, net) {
  const got = await getText(url, net, API_HEADERS);
  if (got.status === 200) {
    try {
      return { json: JSON.parse(got.text), headers: got.headers };
    } catch {
      throw new InstallerError(`GitHub sent an answer that is not JSON for ${url}`);
    }
  }
  if (got.status === 404) return null;
  const limited = got.headers.get('x-ratelimit-remaining') === '0' || got.headers.has('retry-after');
  if ((got.status === 403 || got.status === 429) && limited) {
    throw new InstallerError(
      'the GitHub API rate limit for this network is used up (60 requests per hour without sign-in). ' +
        `Try again ${retryText(got.headers, net.now ?? Date.now)}.`,
    );
  }
  throw new InstallerError(`GitHub answered HTTP ${got.status} for ${url}`);
}

/** The version of a tag "v<version>", or null for another tag. */
export function tagVersion(tag) {
  const text = String(tag ?? '');
  if (!text.startsWith('v')) return null;
  return parseVersion(text.slice(1)) ? text.slice(1) : null;
}

function toRelease(json, latest) {
  const version = tagVersion(json?.tag_name);
  if (!version) throw new InstallerError(`the release tag "${json?.tag_name}" is not v<version>`);
  const assets = (Array.isArray(json.assets) ? json.assets : []).map((a) => ({
    name: String(a?.name ?? ''),
    url: String(a?.browser_download_url ?? ''),
    size: Number(a?.size) || 0,
  }));
  return { tag: json.tag_name, version, prerelease: Boolean(json.prerelease), latest, assets };
}

/**
 * The release of `version` (without "v"), or the latest release when
 * `version` is null: { tag, version, prerelease, latest, assets }.
 */
export async function getRelease(version, net) {
  const url = version ? `${API_URL}/releases/tags/v${version}` : `${API_URL}/releases/latest`;
  const got = await api(url, net);
  if (!got) {
    if (version) {
      throw new InstallerError(`no Pandite release v${version} found at ${RELEASES_URL}. Run "npx pandite --list" for the versions.`);
    }
    throw new InstallerError(`no Pandite release found at ${RELEASES_URL}`);
  }
  return toRelease(got.json, !version);
}

function nextPage(link) {
  const m = /<([^>]+)>;\s*rel="next"/.exec(link ?? '');
  return m && m[1].startsWith('https://api.github.com/') ? m[1] : null;
}

/** Every published release version, newest first: [{ version, prerelease }]. */
export async function listReleases(net) {
  const found = new Map();
  let url = `${API_URL}/releases?per_page=100`;
  for (let page = 0; url && page < MAX_PAGES; page += 1) {
    const got = await api(url, net);
    if (!got) break;
    for (const r of Array.isArray(got.json) ? got.json : []) {
      const version = tagVersion(r?.tag_name);
      if (!version || r.draft) continue;
      found.set(version, { version, prerelease: Boolean(r.prerelease) });
    }
    url = nextPage(got.headers.get('link'));
  }
  return [...found.values()].sort((a, b) => compareVersions(b.version, a.version));
}

/** Parse sha256sum output: Map of file name to lowercase SHA-256. */
export function parseSums(text) {
  const sums = new Map();
  for (const line of String(text).split(/\r?\n/)) {
    const m = /^([0-9a-fA-F]{64}) [ *](.+?)\s*$/.exec(line);
    if (m) sums.set(m[2], m[1].toLowerCase());
  }
  return sums;
}
