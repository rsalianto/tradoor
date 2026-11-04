-- A position may be sized by its margin rather than its risk: `sizeBy` and
-- `margin` in its drawing's style, so moving its stop changes what it risks
-- and not its size. Every position drawn so far goes over at the size it holds
-- now, so no figure moves; its risk settings stay, and setting `sizeBy` back
-- to `risk` picks them up again. A trade keeps its leverage, for its margin.
ALTER TABLE journal_trade ADD COLUMN leverage DOUBLE PRECISION NOT NULL DEFAULT 1;

-- A style value the page may have written as a number or as a string.
CREATE FUNCTION pg_temp.style_num(style JSONB, key TEXT, fallback DOUBLE PRECISION) RETURNS DOUBLE PRECISION AS $$
  SELECT CASE WHEN style->>key ~ '^[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$' AND (style->>key)::DOUBLE PRECISION > 0 THEN (style->>key)::DOUBLE PRECISION ELSE fallback END
$$ LANGUAGE SQL IMMUTABLE;

UPDATE journal_trade t SET leverage = pg_temp.style_num(d.style, 'leverage', 1) FROM drawing d WHERE d.id = t.drawing_id;

WITH sized AS (
  SELECT id, (points->0->>'price')::DOUBLE PRECISION AS entry, (points->1->>'price')::DOUBLE PRECISION AS stop,
    CASE WHEN COALESCE(style->>'riskUnit', '%') = '%' THEN pg_temp.style_num(style, 'account', 1000) * pg_temp.style_num(style, 'risk', 1) / 100 ELSE pg_temp.style_num(style, 'risk', 10) END AS risk,
    pg_temp.style_num(style, 'leverage', 1) AS leverage
  FROM drawing
  WHERE kind IN ('long', 'short') AND NOT style ? 'sizeBy' AND jsonb_array_length(points) >= 3
)
UPDATE drawing d SET style = d.style || jsonb_build_object('sizeBy', 'margin', 'margin', s.risk / abs(s.entry - s.stop) * s.entry / s.leverage)
FROM sized s WHERE d.id = s.id AND s.entry > 0 AND s.entry <> s.stop;
