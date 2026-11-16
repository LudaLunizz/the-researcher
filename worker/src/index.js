// The researcher's opt-in, anonymous usage receiver (Cloudflare Worker + D1).
//
//   POST /v1/usage   body = what `researcher --usage-send` sends (schema 2), or the file
//                    `researcher --usage-export` writes (schema 1)
//   GET  /v1/stats   public totals: installs, loads, characters, estimated tokens
//   GET  /v1/badge/squeezed, /v1/badge/not-loaded   the token totals as shields.io badges
//
// Nothing here reads, logs or stores an IP address, a header or a GitHub login. The
// only data kept is what validate.js lets through: a random install id, a day,
// document counts per format and character sums.

import { MAX_BODY, parsePayload, rowHash } from "./validate.js";

const INSERT =
  "INSERT OR IGNORE INTO loads (h, id, day, docs, pdf, html, md, txt, src_bytes, raw, chars, lib_docs, lib_chars, tk) " +
  "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)";
// Tokens are estimated per record with the characters-per-1000-tokens the app assumed (tk).
const STATS =
  "SELECT COUNT(DISTINCT id) AS installs, COUNT(*) AS loads, COALESCE(SUM(docs), 0) AS docs, " +
  "COALESCE(SUM(raw), 0) AS raw_chars, COALESCE(SUM(chars), 0) AS chars, " +
  "COALESCE(SUM(MAX(raw - chars, 0) * 1000.0 / tk), 0) AS tokens_squeezed, " +
  "COALESCE(SUM(MAX(lib_chars - chars, 0) * 1000.0 / tk), 0) AS tokens_not_loaded FROM loads";
const BADGES = {
  squeezed: { label: "tokens squeezed", key: "tokens_squeezed", color: "brightgreen" },
  "not-loaded": { label: "tokens not loaded", key: "tokens_not_loaded", color: "blue" },
};

/** 1234 -> "1.2k", 2712345 -> "2.7M": short enough for a badge. */
export function short(n) {
  const v = Math.round(n);
  for (const [d, unit] of [[1e9, "B"], [1e6, "M"], [1e3, "k"]]) {
    if (v >= d) {
      const x = v / d;
      return `${x >= 100 ? Math.round(x) : Number(x.toFixed(1))}${unit}`;
    }
  }
  return String(v);
}
const CHUNK = 50; // statements per D1 batch

const HEADERS = { "x-content-type-options": "nosniff" };

function json(body, status = 200, extra = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...HEADERS, "content-type": "application/json; charset=utf-8", ...extra },
  });
}

/** The request body as text, or null when it is bigger than the limit. */
async function readLimited(request, limit) {
  const declared = Number(request.headers.get("content-length"));
  if (Number.isFinite(declared) && declared > limit) return null;
  if (!request.body) return "";
  const reader = request.body.getReader();
  const parts = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > limit) {
      await reader.cancel();
      return null;
    }
    parts.push(value);
  }
  const all = new Uint8Array(size);
  let at = 0;
  for (const p of parts) {
    all.set(p, at);
    at += p.byteLength;
  }
  return new TextDecoder().decode(all);
}

async function receive(request, env) {
  if (!(request.headers.get("content-type") || "").toLowerCase().startsWith("application/json")) {
    return json({ ok: false, error: "send application/json" }, 415);
  }
  const text = await readLimited(request, MAX_BODY);
  if (text === null) return json({ ok: false, error: "body too large" }, 413);
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    return json({ ok: false, error: "not JSON" }, 400);
  }
  const { rows, error } = parsePayload(body);
  if (error) return json({ ok: false, error }, 400);

  const statements = [];
  for (const r of rows) {
    const h = await rowHash(r);
    statements.push(
      env.DB.prepare(INSERT).bind(h, r.id, r.day, r.docs, r.pdf, r.html, r.md, r.txt, r.src_bytes, r.raw, r.chars, r.lib_docs, r.lib_chars, r.tk),
    );
  }
  let added = 0;
  for (let i = 0; i < statements.length; i += CHUNK) {
    const results = await env.DB.batch(statements.slice(i, i + CHUNK));
    for (const res of results) added += res.meta?.changes ?? 0;
  }
  return json({ ok: true, received: rows.length, new: added }, 200, { "cache-control": "no-store" });
}

async function totals(env) {
  const row = await env.DB.prepare(STATS).first();
  return {
    installs: row.installs,
    loads: row.loads,
    documents: row.docs,
    raw_chars: row.raw_chars,
    chars: row.chars,
    tokens_squeezed: Math.round(row.tokens_squeezed),
    tokens_not_loaded: Math.round(row.tokens_not_loaded),
  };
}

async function stats(env) {
  return json(await totals(env), 200, { "cache-control": "public, max-age=300" });
}

/** shields.io "endpoint" JSON: https://img.shields.io/endpoint?url=<this URL> */
async function badge(env, which) {
  const b = BADGES[which];
  const t = await totals(env);
  return json(
    { schemaVersion: 1, label: b.label, message: short(t[b.key]), color: b.color, cacheSeconds: 3600 },
    200,
    { "cache-control": "public, max-age=3600" },
  );
}

export default {
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    if (pathname === "/v1/usage") {
      if (request.method !== "POST") return json({ ok: false, error: "POST only" }, 405, { allow: "POST" });
      // Optional rate limit (see wrangler.toml). The IP is only the key of the limiter; it is never stored here.
      if (env.RATE) {
        const { success } = await env.RATE.limit({ key: request.headers.get("cf-connecting-ip") || "unknown" });
        if (!success) return json({ ok: false, error: "slow down" }, 429, { "retry-after": "60" });
      }
      return receive(request, env);
    }
    if (pathname === "/v1/stats") {
      if (request.method !== "GET") return json({ ok: false, error: "GET only" }, 405, { allow: "GET" });
      return stats(env);
    }
    if (pathname.startsWith("/v1/badge/") && Object.hasOwn(BADGES, pathname.slice(10))) {
      if (request.method !== "GET") return json({ ok: false, error: "GET only" }, 405, { allow: "GET" });
      return badge(env, pathname.slice(10));
    }
    if (pathname === "/") {
      return new Response("the-researcher usage receiver: anonymous counts only. https://github.com/LudaLunizz/the-researcher\n", {
        headers: { ...HEADERS, "content-type": "text/plain; charset=utf-8" },
      });
    }
    return json({ ok: false, error: "not found" }, 404);
  },
};
