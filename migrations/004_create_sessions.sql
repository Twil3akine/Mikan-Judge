CREATE TABLE IF NOT EXISTS tower_sessions (
    id          TEXT   PRIMARY KEY,
    data        TEXT   NOT NULL,
    expiry_unix BIGINT NOT NULL
);
