-- For a database made before record version 2 (researcher 0.5): per-format characters and
-- chat loads. Old rows keep NULL characters (unknown) and 0 chats. Run once:
--   npx wrangler d1 execute researcher-usage --remote --file migrations/0002_formats.sql
ALTER TABLE loads ADD COLUMN chat       INTEGER NOT NULL DEFAULT 0;
ALTER TABLE loads ADD COLUMN pdf_chars  INTEGER;
ALTER TABLE loads ADD COLUMN html_chars INTEGER;
ALTER TABLE loads ADD COLUMN md_chars   INTEGER;
ALTER TABLE loads ADD COLUMN txt_chars  INTEGER;
ALTER TABLE loads ADD COLUMN chat_chars INTEGER;
