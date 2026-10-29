-- A footprint's history: volume by price inside each hour, kept for good. The
-- minutes go after a month (V12); before they do, each hour of them is added up
-- into a row here, and hours from before the server recorded can be read from
-- an exchange's archive of every trade. A chart of hours, days or weeks is
-- built from these: a day is twenty-four rows rather than fourteen hundred.
--
-- The same packing as a minute, and how far the delta swung inside the hour
-- from its open, read at each minute's close — in coins and in dollars — so a
-- day added up from hours still knows the most and least its delta reached.
CREATE TABLE footprint_hour (
    market         TEXT NOT NULL,
    hour           BIGINT NOT NULL,
    tick           DOUBLE PRECISION NOT NULL,
    base           BIGINT NOT NULL,
    levels         BYTEA NOT NULL,
    trades         BIGINT NOT NULL,
    swing_up       DOUBLE PRECISION NOT NULL,
    swing_down     DOUBLE PRECISION NOT NULL,
    swing_up_usd   DOUBLE PRECISION NOT NULL,
    swing_down_usd DOUBLE PRECISION NOT NULL,
    -- Read from the exchange's archive, which misses no trade; otherwise added
    -- up from the minutes recorded, which miss what a restart missed.
    archived       BOOLEAN NOT NULL,
    PRIMARY KEY (market, hour)
);

-- The archive files already read, a month or a day each, so a restart carries
-- on where it was rather than reading them all again.
CREATE TABLE footprint_archive (
    market  TEXT NOT NULL,
    period  TEXT NOT NULL,
    hours   INTEGER NOT NULL,
    read_at BIGINT NOT NULL,
    PRIMARY KEY (market, period)
);
