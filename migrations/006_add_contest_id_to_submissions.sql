ALTER TABLE submissions
    ADD COLUMN contest_id TEXT REFERENCES contests(id) ON DELETE SET NULL;

CREATE INDEX submissions_contest_id_idx ON submissions(contest_id);
