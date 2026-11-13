-- What the site's owner tells every reader, from the status page: the latest
-- that has not been taken down is shown, a banner atop the app. Kept, ended
-- or not, as a record of what was said and when.
CREATE TABLE announcement (
    id         BIGINT PRIMARY KEY,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL,
    link       TEXT,
    tone       TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    ended_at   BIGINT
);
