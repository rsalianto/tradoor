-- Frost Scripts: the indicators accounts write (crates/formula), kept as their
-- text and run only by the formula module, in each reader's browser. A script
-- the moderator removes is kept, with why, so a mistake can be put right; it
-- runs for no one, its author included, until it is restored.
CREATE TABLE frost_script (
    id          TEXT PRIMARY KEY,
    owner       TEXT NOT NULL,
    name        TEXT NOT NULL,
    source      TEXT NOT NULL,
    checks      BOOLEAN NOT NULL,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL,
    removed_at  BIGINT,
    removed_why TEXT
);
CREATE INDEX frost_script_owner_idx ON frost_script (owner, updated_at DESC);
CREATE INDEX frost_script_updated_idx ON frost_script (updated_at DESC);
