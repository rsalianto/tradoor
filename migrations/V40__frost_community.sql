-- Frost Scripts shared with everyone: a script is private, its author's alone,
-- until its author makes it public. A public one is listed in the indicator
-- picker's Community tab under its author's username, and can be added to any
-- chart; made private again, or removed by the moderator, it runs for its
-- author alone (removed: for no one).
ALTER TABLE frost_script ADD COLUMN public BOOLEAN NOT NULL DEFAULT false;
-- What it shows and how to read it, in its author's words: needed to share it.
ALTER TABLE frost_script ADD COLUMN about TEXT NOT NULL DEFAULT '';
-- The chart it was shared from — market, timeframe and its settings there — for
-- the showcase to draw it on, live, with tradoor's own engine: no picture kept.
ALTER TABLE frost_script ADD COLUMN preview_market TEXT;
ALTER TABLE frost_script ADD COLUMN preview_timeframe TEXT;
ALTER TABLE frost_script ADD COLUMN preview_inputs TEXT;
CREATE INDEX frost_script_public_idx ON frost_script (updated_at DESC) WHERE public AND removed_at IS NULL;

-- An account's username, chosen by the account in place of its Telegram name,
-- which it need not give away. Chosen once and never changed, bound to the
-- Telegram account: a name seen on a shared script always means the same
-- author. One of each, whatever its capitals.
CREATE TABLE account_username (
    owner         TEXT PRIMARY KEY,
    telegram_user BIGINT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    created_at    BIGINT NOT NULL
);
CREATE UNIQUE INDEX account_username_name_idx ON account_username (lower(name));
