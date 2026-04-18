INSERT INTO contests (id, title, description, start_time, end_time)
VALUES (
    'welcome-mikan',
    'Welcome to MikanJudge!',
    'MikanJudge へようこそ！このコンテストではプログラミングの基礎を楽しみましょう。',
    '2020-01-01 00:00:00+00',
    '2099-12-31 23:59:59+00'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO contest_problems (contest_id, problem_id, display_order, label)
VALUES ('welcome-mikan', 'aplusb', 1, 'A')
ON CONFLICT (contest_id, problem_id) DO NOTHING;
