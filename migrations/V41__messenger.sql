-- tradoor messenger: one room every reader sees and anyone may write in.
-- The newest messages are kept, so one who opens it sees the talk so far; the
-- server lets the oldest go past a couple of hundred. A message keeps its
-- sender's browser id, never shown, so the moderator can mute them.
CREATE TABLE messenger_message (
    id     BIGSERIAL PRIMARY KEY,
    owner  TEXT NOT NULL,
    name   TEXT NOT NULL,
    signed BOOLEAN NOT NULL,
    text   TEXT NOT NULL,
    at     BIGINT NOT NULL
);

-- Senders the moderator silenced, by browser id, until when.
CREATE TABLE messenger_mute (
    owner TEXT PRIMARY KEY,
    until BIGINT NOT NULL,
    why   TEXT
);
