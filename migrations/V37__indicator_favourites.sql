-- The indicators an account starred in the picker, in the order it starred them.
-- A row of its own rather than a field of the workspace, which is saved whole and
-- whichever copy changed last wins: a star on the phone must not be lost to a
-- desktop tab that was open all along.
CREATE TABLE indicator_favourite (
    owner    TEXT NOT NULL,
    kind     TEXT NOT NULL,
    added_at BIGINT NOT NULL,
    PRIMARY KEY (owner, kind)
);
