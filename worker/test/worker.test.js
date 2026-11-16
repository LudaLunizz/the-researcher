// node --test test/   (Node 22.13+; the D1 stand-in is node:sqlite running the real schema.sql)
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";

import worker from "../src/index.js";
import { MAX_LOADS, parsePayload, recordError, rowHash } from "../src/validate.js";

const NOW = Date.UTC(2026, 10, 16, 12);
const record = (over = {}) => ({
  chars: 69652,
  day: "2026-11-16",
  docs: 7,
  fmt: { html: 4, md: 3, pdf: 0, txt: 0 },
  id: "7d75b3c23f2e4970",
  lib_chars: 2239282,
  lib_docs: 114,
  raw: 69655,
  src_bytes: 113432,
  tk: 2300,
  v: 1,
  ...over,
});
const payload = (...loads) => ({ schema: 1, loads: loads.length ? loads : [record()] });
/** What `researcher --usage-send` sends: schema 2, every record numbered by its log line. */
const sent = (...loads) => ({ schema: 2, loads: loads.map((r, i) => ({ ...r, seq: r.seq ?? i + 1 })) });

/** The few D1 calls the Worker uses, on an in-memory SQLite with the real schema. */
function fakeD1() {
  const db = new DatabaseSync(":memory:");
  db.exec(readFileSync(new URL("../schema.sql", import.meta.url), "utf8"));
  const make = (sql) => ({
    sql,
    args: [],
    bind(...a) {
      this.args = a;
      return this;
    },
    async first() {
      return db.prepare(sql).get(...this.args) ?? null;
    },
  });
  return {
    db,
    prepare: make,
    async batch(stmts) {
      return stmts.map((s) => ({ meta: { changes: Number(db.prepare(s.sql).run(...s.args).changes) } }));
    },
  };
}

const post = (body, env, headers = {}) =>
  worker.fetch(
    new Request("https://x.test/v1/usage", {
      method: "POST",
      headers: { "content-type": "application/json", ...headers },
      body: typeof body === "string" ? body : JSON.stringify(body),
    }),
    env,
  );

test("a real export record is accepted", () => {
  assert.equal(recordError(record(), NOW), null);
  const { rows, error } = parsePayload(payload(), NOW);
  assert.equal(error, undefined);
  assert.equal(rows[0].md, 3);
});

test("anything outside the whitelist is refused", () => {
  const bad = [
    record({ title: "My research" }),
    record({ path: "/home/me/x.html" }),
    record({ id: "not-an-id-at-all!" }),
    record({ id: "7D75B3C23F2E4970" }),
    record({ day: "2026-13-40" }),
    record({ day: "2025-12-31" }),
    record({ day: "2027-06-01" }),
    record({ docs: 8 }), // fmt must add up
    record({ fmt: { html: 4, md: 3, pdf: 0 } }),
    record({ fmt: { html: 4, md: 3, pdf: 0, txt: 0, other: 1 } }),
    record({ chars: -1 }),
    record({ chars: 1.5 }),
    record({ raw: "69655" }),
    record({ tk: 0 }),
    record({ v: 2 }),
  ];
  for (const rec of bad) assert.notEqual(recordError(rec, NOW), null, JSON.stringify(rec));
  assert.ok(parsePayload({ schema: 1, loads: [], extra: 1 }, NOW).error);
  assert.ok(parsePayload({ schema: 2, loads: [record()] }, NOW).error);
  assert.ok(parsePayload({ schema: 1, loads: [] }, NOW).error);
  assert.ok(parsePayload({ schema: 1, loads: Array(MAX_LOADS + 1).fill(record()) }, NOW).error);
});

test("schema 2 needs a seq on every record, schema 1 allows none", () => {
  assert.equal(parsePayload(sent(record()), NOW).error, undefined);
  assert.equal(parsePayload(sent(record()), NOW).rows[0].seq, 1);
  assert.equal(parsePayload(payload(), NOW).rows[0].seq, null);
  assert.ok(parsePayload({ schema: 2, loads: [record()] }, NOW).error); // no seq
  assert.ok(parsePayload({ schema: 1, loads: [record({ seq: 1 })] }, NOW).error); // seq in schema 1
  for (const seq of [0, -1, 1.5, "3", null, 1_000_000_001]) {
    assert.ok(parsePayload({ schema: 2, loads: [record({ seq })] }, NOW).error, String(seq));
  }
  assert.ok(parsePayload({ schema: 3, loads: [record({ seq: 1 })] }, NOW).error);
});

test("schema 1 rows keep the hash they had before seq existed", async () => {
  const { rows } = parsePayload(payload(), NOW);
  const old = createHash("sha256")
    .update("7d75b3c23f2e4970|2026-11-16|7|0|4|3|0|113432|69655|69652|114|2239282|2300")
    .digest("hex");
  assert.equal(await rowHash(rows[0]), old);
  assert.notEqual(await rowHash(parsePayload(sent(record()), NOW).rows[0]), old);
});

test("two identical loads with their own seq are both kept; a resend is stored once", async () => {
  const env = { DB: fakeD1() };
  let res = await post(sent(record(), record()), env);
  assert.deepEqual(await res.json(), { ok: true, received: 2, new: 2 });
  res = await post(sent(record(), record()), env);
  assert.deepEqual(await res.json(), { ok: true, received: 2, new: 0 });
  res = await post(sent(record({ seq: 3 })), env);
  assert.deepEqual(await res.json(), { ok: true, received: 1, new: 1 });
  assert.equal(env.DB.db.prepare("SELECT COUNT(*) AS n FROM loads").get().n, 3);
});

test("POST stores records and the same record twice is stored once", async () => {
  const env = { DB: fakeD1() };
  let res = await post(payload(record(), record({ chars: 100, raw: 120 })), env);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true, received: 2, new: 2 });

  res = await post(payload(record()), env);
  assert.deepEqual(await res.json(), { ok: true, received: 1, new: 0 });

  const row = env.DB.db.prepare("SELECT COUNT(*) AS n, SUM(docs) AS docs FROM loads").get();
  assert.equal(row.n, 2);
  assert.equal(row.docs, 14);
});

test("a bad record stores nothing, not even the good ones", async () => {
  const env = { DB: fakeD1() };
  const res = await post(payload(record(), record({ title: "x" })), env);
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /^record 1:/);
  assert.equal(env.DB.db.prepare("SELECT COUNT(*) AS n FROM loads").get().n, 0);
});

test("wrong method, content type, JSON and size", async () => {
  const env = { DB: fakeD1() };
  let res = await worker.fetch(new Request("https://x.test/v1/usage"), env);
  assert.equal(res.status, 405);

  res = await post(payload(), env, { "content-type": "text/plain" });
  assert.equal(res.status, 415);

  res = await post("{not json", env);
  assert.equal(res.status, 400);

  res = await post("x".repeat(300_000), env);
  assert.equal(res.status, 413);
});

test("rate limit answers 429 and stores nothing", async () => {
  const env = { DB: fakeD1(), RATE: { limit: async () => ({ success: false }) } };
  const res = await post(payload(), env);
  assert.equal(res.status, 429);
  assert.equal(env.DB.db.prepare("SELECT COUNT(*) AS n FROM loads").get().n, 0);
});

test("stats are totals only", async () => {
  const env = { DB: fakeD1() };
  await post(payload(record(), record({ id: "0123456789abcdef", chars: 10, raw: 20 })), env);
  const res = await worker.fetch(new Request("https://x.test/v1/stats"), env);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), {
    installs: 2,
    loads: 2,
    documents: 14,
    raw_chars: 69655 + 20,
    chars: 69652 + 10,
  });
});

test("nothing else is served", async () => {
  const env = { DB: fakeD1() };
  assert.equal((await worker.fetch(new Request("https://x.test/"), env)).status, 200);
  assert.equal((await worker.fetch(new Request("https://x.test/admin"), env)).status, 404);
});
