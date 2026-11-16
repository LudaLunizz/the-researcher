# Usage receiver (Cloudflare Worker)

The receiving end for the researcher's **opt-in, anonymous** usage counts. The app
never sends anything today; this is the place it will send to once sending is added
(see the issue tracker). No dependencies: plain JavaScript, a D1 database, nothing else.

## What it accepts

`POST /v1/usage` with `Content-Type: application/json` and exactly the file
`researcher --usage-export` writes:

```json
{"schema": 1, "loads": [
  {"v": 1, "day": "2026-11-16", "id": "7d75b3c23f2e4970", "docs": 7,
   "fmt": {"html": 4, "md": 3, "pdf": 0, "txt": 0},
   "src_bytes": 113432, "raw": 69655, "chars": 69652,
   "lib_docs": 114, "lib_chars": 2239282, "tk": 2400}
]}
```

Validation is a strict whitelist (`src/validate.js`): any other field, any string that
is not a 16-hex id or a plausible date, any out-of-range number, or a `fmt` that does
not add up to `docs` rejects the **whole** request and stores nothing. Free text
(titles, paths, names) cannot get in. Limits: 200 records and 256 KB per request.

A record that is sent twice is stored once (primary key = hash of its numbers), so the
app can resend its queue safely.

`GET /v1/stats` returns public totals only: installs, loads, documents, characters.

## What it keeps and does not keep

Kept: the random install id, the day, document counts per format, character sums.

Not kept: IP address, headers, user agent, titles, paths, GitHub login. The code never
reads them (the optional rate limiter uses the IP only as its key), and
`observability` is off in `wrangler.toml`, so Cloudflare keeps no request logs for this
Worker. Cloudflare itself still sees connections as any host does.

## Deploy

```sh
cd worker
npx wrangler login
npx wrangler d1 create researcher-usage      # prints database_id -> put it in wrangler.toml
npx wrangler d1 execute researcher-usage --remote --file schema.sql
npx wrangler deploy                          # prints https://the-researcher-usage.<you>.workers.dev
```

Read the numbers yourself:

```sh
npx wrangler d1 execute researcher-usage --remote --command \
  "SELECT day, COUNT(DISTINCT id) AS installs, COUNT(*) AS loads, SUM(chars) AS chars FROM loads GROUP BY day ORDER BY day DESC LIMIT 14"
```

Delete one install's data on request: `DELETE FROM loads WHERE id = '<their id>'`.

Optional: switch on the rate limit block in `wrangler.toml` (not tested here; check
Cloudflare's current "Rate Limiting" docs for the syntax first).

## Test

Node 22.13 or newer (the D1 stand-in is `node:sqlite` running the real `schema.sql`):

```sh
cd worker
npm test
```
