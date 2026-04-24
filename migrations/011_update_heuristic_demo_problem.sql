DELETE FROM contest_problems
WHERE contest_id = 'heuristic-demo' AND problem_id = 'closest_sum';

INSERT INTO contest_problems (contest_id, problem_id, display_order, label)
VALUES ('heuristic-demo', 'max_cut', 1, 'A')
ON CONFLICT (contest_id, problem_id) DO UPDATE SET
    display_order = EXCLUDED.display_order,
    label = EXCLUDED.label;
