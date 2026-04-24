#!/usr/bin/env python3
import sys


MASK = 0xFFFFFFFF


def xorshift32(x: int) -> int:
    x ^= (x << 13) & MASK
    x ^= x >> 17
    x ^= (x << 5) & MASK
    return x & MASK


def generate_graph(n: int, m: int, seed: int):
    x = seed & MASK
    total = 0
    edges = []
    for _ in range(m):
        x = xorshift32(x)
        a = x % n
        x = xorshift32(x)
        b = x % (n - 1)
        if b >= a:
            b += 1
        x = xorshift32(x)
        w = 1 + (x % 1000)
        edges.append((a, b, w))
        total += w
    return edges, total


def score(input_path: str, output_path: str) -> int:
    with open(input_path, "r", encoding="utf-8") as f:
        n, m, seed = map(int, f.read().split())

    try:
        with open(output_path, "r", encoding="utf-8") as f:
            out = f.read().split()
        colors = list(map(int, out))
    except Exception:
        return 0

    if len(colors) != n:
        return 0
    if any(c not in (0, 1) for c in colors):
        return 0

    edges, total = generate_graph(n, m, seed)
    cut = 0
    for a, b, w in edges:
        if colors[a] != colors[b]:
            cut += w

    return round(1_000_000 * cut / total)


if __name__ == "__main__":
    print(score(sys.argv[1], sys.argv[2]))
