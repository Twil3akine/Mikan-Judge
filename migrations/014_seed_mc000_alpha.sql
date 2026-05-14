INSERT INTO contests (id, title, description, start_time, end_time, judge_type)
VALUES (
    'mc000-alpha',
    'MC000-alpha',
    'Alpha 版の 5 問コンテストです。A から E に向かって段階的に難しくなります。',
    '2026-04-30 18:00:00+09',
    '2026-04-30 19:00:00+09',
    'exact'
)
ON CONFLICT (id) DO UPDATE SET
    title = EXCLUDED.title,
    description = EXCLUDED.description,
    start_time = EXCLUDED.start_time,
    end_time = EXCLUDED.end_time,
    judge_type = EXCLUDED.judge_type;

INSERT INTO contest_problems (contest_id, problem_id, display_order, label)
VALUES
    ('mc000-alpha', 'mc000_alpha_a', 1, 'A'),
    ('mc000-alpha', 'mc000_alpha_b', 2, 'B'),
    ('mc000-alpha', 'mc000_alpha_c', 3, 'C'),
    ('mc000-alpha', 'mc000_alpha_d', 4, 'D'),
    ('mc000-alpha', 'mc000_alpha_e', 5, 'E')
ON CONFLICT (contest_id, problem_id) DO NOTHING;
