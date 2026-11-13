// HTTPS requests through an injected fetch function: redirects to HTTPS
// addresses only, a time limit without data, size limits, and the SHA-256
// of each download.
//
// `net` = { fetch, userAgent, idleMs }. The fetch function has the
// signature of the global fetch of Node.js 18 and later.

import { createHash } from 'node:crypto';
import { createWriteStream, rmSync } from 'node:fs';
import { Readable, Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { InstallerError } from './errors.js';

export const MAX_REDIRECTS = 5;
export const MAX_DOWNLOAD_BYTES = 1024 * 1024 * 1024;
export const MAX_TEXT_BYTES = 16 * 1024 * 1024;
export const IDLE_TIMEOUT_MS = 60_000;
const REDIRECT_CODES = new Set([301, 302, 303, 307, 308]);

function describe(error) {
  const cause = error?.cause;
  const code = cause?.code ?? error?.code;
  const text = cause?.message ?? error?.message ?? String(error);
  return code && !String(text).includes(code) ? `${code} ${text}` : String(text);
}

/** Aborts `controller` after `ms` milliseconds without a call to touch(). */
function idleTimer(controller, ms) {
  const state = { expired: false };
  let timer = null;
  const touch = () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      state.expired = true;
      controller.abort();
    }, ms);
    timer.unref?.();
  };
  touch();
  return { state, touch, stop: () => clearTimeout(timer) };
}

async function discard(res) {
  try {
    await res.body?.cancel?.();
  } catch {
    // The body is not needed.
  }
}

/**
 * GET `url`, following at most MAX_REDIRECTS redirects, each to an HTTPS
 * address only. Resolves { res, url } for the first answer that is not a
 * redirect. The caller reads or discards the body.
 */
async function get(url, net, { headers, controller, idle }) {
  const idleMs = net.idleMs ?? IDLE_TIMEOUT_MS;
  let current = new URL(url);
  for (let hops = 0; ; hops += 1) {
    if (current.protocol !== 'https:') {
      const what = hops === 0 ? current.href : `the redirect to ${current.href}`;
      throw new InstallerError(`refusing ${what}: only HTTPS addresses are allowed`);
    }
    let res;
    try {
      res = await net.fetch(current.href, {
        headers: { 'user-agent': net.userAgent, ...headers },
        redirect: 'manual',
        signal: controller.signal,
      });
    } catch (error) {
      if (idle.state.expired) throw new InstallerError(`no answer from ${current.host} for ${idleMs / 1000} seconds`);
      throw new InstallerError(`cannot reach ${current.host}: ${describe(error)}. Check the network connection.`);
    }
    idle.touch();
    if (REDIRECT_CODES.has(res.status)) {
      await discard(res);
      const location = res.headers.get('location');
      if (!location) throw new InstallerError(`${current.href} answered HTTP ${res.status} without a Location header`);
      if (hops >= MAX_REDIRECTS) throw new InstallerError(`more than ${MAX_REDIRECTS} redirects for ${url}`);
      current = new URL(location, current);
      continue;
    }
    return { res, url: current.href };
  }
}

/** GET a text resource: { status, headers, text }. Any HTTP status resolves. */
export async function getText(url, net, headers = {}) {
  const idleMs = net.idleMs ?? IDLE_TIMEOUT_MS;
  const controller = new AbortController();
  const idle = idleTimer(controller, idleMs);
  try {
    const { res } = await get(url, net, { headers, controller, idle });
    const chunks = [];
    let bytes = 0;
    try {
      for await (const chunk of res.body ?? []) {
        bytes += chunk.length;
        if (bytes > MAX_TEXT_BYTES) {
          await discard(res);
          throw new InstallerError(`${url} sent more than ${MAX_TEXT_BYTES} bytes`);
        }
        idle.touch();
        chunks.push(Buffer.from(chunk));
      }
    } catch (error) {
      if (error instanceof InstallerError) throw error;
      if (idle.state.expired) throw new InstallerError(`no data from ${new URL(url).host} for ${idleMs / 1000} seconds`);
      throw new InstallerError(`reading ${url} failed: ${describe(error)}`);
    }
    return { status: res.status, headers: res.headers, text: Buffer.concat(chunks).toString('utf8') };
  } finally {
    idle.stop();
  }
}

/**
 * Download `url` into the new file `file`. Resolves { sha256, bytes }.
 * Deletes the file when the download fails.
 */
export async function download(url, file, net, { maxBytes = MAX_DOWNLOAD_BYTES } = {}) {
  const idleMs = net.idleMs ?? IDLE_TIMEOUT_MS;
  const controller = new AbortController();
  const idle = idleTimer(controller, idleMs);
  try {
    const { res } = await get(url, net, { headers: { accept: 'application/octet-stream' }, controller, idle });
    if (res.status !== 200 || !res.body) {
      await discard(res);
      const hint = res.status === 404 ? ' (the file does not exist)' : '';
      throw new InstallerError(`download of ${url} failed: HTTP ${res.status}${hint}`);
    }
    const hash = createHash('sha256');
    let bytes = 0;
    const meter = new Transform({
      transform(chunk, _encoding, done) {
        bytes += chunk.length;
        if (bytes > maxBytes) {
          done(new InstallerError(`${url} is larger than ${maxBytes} bytes`));
          return;
        }
        idle.touch();
        hash.update(chunk);
        done(null, chunk);
      },
    });
    try {
      await pipeline(Readable.from(res.body), meter, createWriteStream(file, { flags: 'wx' }));
    } catch (error) {
      if (error?.code !== 'EEXIST') rmSync(file, { force: true });
      if (error instanceof InstallerError) throw error;
      if (idle.state.expired) throw new InstallerError(`no data from ${new URL(url).host} for ${idleMs / 1000} seconds`);
      throw new InstallerError(`download of ${url} failed: ${describe(error)}`);
    }
    return { sha256: hash.digest('hex'), bytes };
  } finally {
    idle.stop();
  }
}
