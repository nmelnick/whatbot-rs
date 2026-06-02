CREATE TABLE person (
    id          BIGSERIAL PRIMARY KEY,
    display     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE account (
    id          BIGSERIAL PRIMARY KEY,
    service     TEXT NOT NULL,
    handle      TEXT NOT NULL,
    display     TEXT NOT NULL,
    person_id   BIGINT NULL REFERENCES person(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (service, handle)
);

CREATE INDEX account_person_id_idx ON account(person_id);

CREATE TABLE account_capability (
    account_id  BIGINT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    capability  TEXT NOT NULL,
    PRIMARY KEY (account_id, capability)
);

CREATE TABLE factoid (
    id           BIGSERIAL PRIMARY KEY,
    subject      TEXT NOT NULL,
    subject_norm TEXT NOT NULL UNIQUE,
    is_plural    BOOLEAN NOT NULL DEFAULT FALSE,
    is_or        BOOLEAN NOT NULL DEFAULT FALSE,
    silent       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE factoid_fact (
    id           BIGSERIAL PRIMARY KEY,
    factoid_id   BIGINT NOT NULL REFERENCES factoid(id) ON DELETE CASCADE,
    description  TEXT NOT NULL,
    account_id   BIGINT NULL REFERENCES account(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX factoid_fact_factoid_id_idx ON factoid_fact(factoid_id);

CREATE TABLE karma (
    id           BIGSERIAL PRIMARY KEY,
    subject      TEXT NOT NULL,
    subject_norm TEXT NOT NULL,
    delta        INTEGER NOT NULL,
    account_id   BIGINT NULL REFERENCES account(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX karma_subject_norm_idx ON karma(subject_norm);
CREATE INDEX karma_account_id_idx ON karma(account_id);
