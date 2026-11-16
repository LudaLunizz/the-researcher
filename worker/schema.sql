-- One row per load. No names, no paths, no IP addresses, no GitHub login.
-- `h` is a hash of the numbers, so a record that is sent twice is stored once.
CREATE TABLE IF NOT EXISTS loads (
  h         TEXT PRIMARY KEY,
  id        TEXT NOT NULL,    -- random install id (16 hex), reset by deleting the usage-id file
  day       TEXT NOT NULL,    -- YYYY-MM-DD, local day of the load
  docs      INTEGER NOT NULL,
  pdf       INTEGER NOT NULL,
  html      INTEGER NOT NULL,
  md        INTEGER NOT NULL,
  txt       INTEGER NOT NULL,
  src_bytes INTEGER NOT NULL,
  raw       INTEGER NOT NULL, -- characters before squeezing
  chars     INTEGER NOT NULL, -- characters loaded
  lib_docs  INTEGER NOT NULL,
  lib_chars INTEGER NOT NULL,
  tk        INTEGER NOT NULL  -- characters per 1000 tokens the app assumed
);
CREATE INDEX IF NOT EXISTS loads_day ON loads (day);
CREATE INDEX IF NOT EXISTS loads_id ON loads (id);
