INSERT INTO contests (id, title, description, start_time, end_time, judge_type)
VALUES (
    'heuristic-demo',
    'MikanJudge Heuristic Demo',
    'ヒューリスティック形式の動作確認用コンテストです。',
    '2026-01-01 00:00:00+00',
    '2099-12-31 23:59:59+00',
    'heuristic'
)
ON CONFLICT (id) DO UPDATE SET
    title = EXCLUDED.title,
    description = EXCLUDED.description,
    start_time = EXCLUDED.start_time,
    end_time = EXCLUDED.end_time,
    judge_type = EXCLUDED.judge_type;

INSERT INTO contest_problems (contest_id, problem_id, display_order, label)
VALUES ('heuristic-demo', 'closest_sum', 1, 'A')
ON CONFLICT (contest_id, problem_id) DO NOTHING;
