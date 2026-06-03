CREATE TABLE seen (
    id          BIGSERIAL PRIMARY KEY,
    handle      TEXT NOT NULL,
    handle_norm TEXT NOT NULL UNIQUE,
    message     TEXT NOT NULL DEFAULT '',
    seen_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
