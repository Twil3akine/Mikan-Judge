CREATE TABLE contests (
    id          TEXT        PRIMARY KEY,
    title       TEXT        NOT NULL,
    description TEXT        NOT NULL DEFAULT '',
    start_time  TIMESTAMPTZ NOT NULL,
    end_time    TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE contest_problems (
    contest_id    TEXT NOT NULL REFERENCES contests(id) ON DELETE CASCADE,
    problem_id    TEXT NOT NULL,
    display_order INT  NOT NULL DEFAULT 0,
    label         TEXT NOT NULL,        -- "A", "B", "C", ...
    PRIMARY KEY (contest_id, problem_id)
);
