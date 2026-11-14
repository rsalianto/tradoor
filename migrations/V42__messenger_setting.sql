-- tradoor messenger's settings, set by the moderator on the status page:
-- `accounts_only` = 'true' lets only signed-in accounts with a username write,
-- the brake for a flood of guests.
CREATE TABLE messenger_setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
