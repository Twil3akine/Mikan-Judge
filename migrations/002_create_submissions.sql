CREATE TABLE submissions (
    id             UUID        PRIMARY KEY,
    problem_id     TEXT        NOT NULL,
    user_id        UUID        REFERENCES users(id),
    language       TEXT        NOT NULL,
    source_code    TEXT        NOT NULL,
    status         TEXT        NOT NULL DEFAULT 'pending',
    time_used_ms   BIGINT,
    memory_used_kb BIGINT,
    stdout         TEXT,
    stderr         TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX submissions_problem_id_idx ON submissions(problem_id);
CREATE INDEX submissions_user_id_idx    ON submissions(user_id);
CREATE INDEX submissions_created_at_idx ON submissions(created_at DESC);
