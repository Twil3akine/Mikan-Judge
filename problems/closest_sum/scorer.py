#!/usr/bin/env python3
import sys


def score(input_path: str, output_path: str) -> int:
    with open(input_path, "r", encoding="utf-8") as f:
        tokens = f.read().split()
    n = int(tokens[0])
    target = int(tokens[1])
    values = list(map(int, tokens[2:2 + n]))

    try:
        with open(output_path, "r", encoding="utf-8") as f:
            out = f.read().split()
        if not out:
            return 0
        k = int(out[0])
        indices = list(map(int, out[1:]))
    except Exception:
        return 0

    if k < 0 or k != len(indices):
        return 0

    seen = set()
    total = 0
    for idx in indices:
        if idx < 1 or idx > n or idx in seen:
            return 0
        seen.add(idx)
        total += values[idx - 1]

    if total > target:
        return 0

    return round(1_000_000 * total / target)


if __name__ == "__main__":
    print(score(sys.argv[1], sys.argv[2]))
