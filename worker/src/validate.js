// Strict validation of the anonymous usage records the researcher writes
// (`researcher --usage-export`). Only these numbers are accepted; any other key,
// any string that is not the expected shape, and any out-of-range number makes the
// whole request fail, so free text (titles, paths, names) can never be stored.

export const SCHEMA = 1;
export const MAX_LOADS = 200; // records per request
export const MAX_BODY = 262144; // bytes per request

const KINDS = ["pdf", "html", "md", "txt"];
const LOAD_KEYS = ["chars", "day", "docs", "fmt", "id", "lib_chars", "lib_docs", "raw", "src_bytes", "tk", "v"];
const COUNT_MAX = 1_000_000; // documents
const SIZE_MAX = 10_000_000_000; // characters or bytes
const DAY_FIRST = Date.UTC(2026, 0, 1);
const DAY_SLACK = 2 * 86_400_000; // clocks and time zones

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const isInt = (v, max) => Number.isSafeInteger(v) && v >= 0 && v <= max;
const sameKeys = (obj, keys) => {
  const have = Object.keys(obj).sort();
  return have.length === keys.length && have.every((k, i) => k === keys[i]);
};

function validDay(s, now) {
  if (typeof s !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(s)) return false;
  const t = Date.parse(`${s}T00:00:00Z`);
  if (Number.isNaN(t) || new Date(t).toISOString().slice(0, 10) !== s) return false;
  return t >= DAY_FIRST && t <= now + DAY_SLACK;
}

/** One record -> error text, or null when it is fine. */
export function recordError(rec, now = Date.now()) {
  if (!isObject(rec) || !sameKeys(rec, LOAD_KEYS)) return "unexpected fields";
  if (rec.v !== SCHEMA) return "unsupported record version";
  if (typeof rec.id !== "string" || !/^[0-9a-f]{16}$/.test(rec.id)) return "bad id";
  if (!validDay(rec.day, now)) return "bad day";
  if (!isInt(rec.docs, COUNT_MAX) || !isInt(rec.lib_docs, COUNT_MAX)) return "bad document count";
  for (const k of ["src_bytes", "raw", "chars", "lib_chars"]) {
    if (!isInt(rec[k], SIZE_MAX)) return `bad ${k}`;
  }
  if (!Number.isSafeInteger(rec.tk) || rec.tk < 1 || rec.tk > 100_000) return "bad tk";
  if (!isObject(rec.fmt) || !sameKeys(rec.fmt, [...KINDS].sort())) return "bad fmt";
  let sum = 0;
  for (const k of KINDS) {
    if (!isInt(rec.fmt[k], COUNT_MAX)) return "bad fmt count";
    sum += rec.fmt[k];
  }
  if (sum !== rec.docs) return "fmt does not add up to docs";
  return null;
}

/** The request body (parsed JSON) -> {rows} or {error}. */
export function parsePayload(body, now = Date.now()) {
  if (!isObject(body) || !sameKeys(body, ["loads", "schema"])) return { error: "unexpected fields" };
  if (body.schema !== SCHEMA) return { error: "unsupported schema" };
  if (!Array.isArray(body.loads) || body.loads.length === 0) return { error: "no records" };
  if (body.loads.length > MAX_LOADS) return { error: `more than ${MAX_LOADS} records` };
  const rows = [];
  for (const [i, rec] of body.loads.entries()) {
    const err = recordError(rec, now);
    if (err) return { error: `record ${i}: ${err}` };
    rows.push({
      id: rec.id,
      day: rec.day,
      docs: rec.docs,
      pdf: rec.fmt.pdf,
      html: rec.fmt.html,
      md: rec.fmt.md,
      txt: rec.fmt.txt,
      src_bytes: rec.src_bytes,
      raw: rec.raw,
      chars: rec.chars,
      lib_docs: rec.lib_docs,
      lib_chars: rec.lib_chars,
      tk: rec.tk,
    });
  }
  return { rows };
}

/** A hash of the record's numbers: sending the same record again changes nothing. */
export async function rowHash(row) {
  const text = [
    row.id, row.day, row.docs, row.pdf, row.html, row.md, row.txt,
    row.src_bytes, row.raw, row.chars, row.lib_docs, row.lib_chars, row.tk,
  ].join("|");
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
