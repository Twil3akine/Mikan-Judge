ALTER TABLE submissions
    DROP CONSTRAINT submissions_user_id_fkey,
    ADD CONSTRAINT submissions_user_id_fkey
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL;
