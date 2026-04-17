# MikanJudge 🍊

Rust 製の競技プログラミング用オンラインジャッジ。

## 機能

- C++17 / Rust / Python3 (CPython) / Python3 (PyPy) での提出・ジャッジ
- fork + exec サンドボックス（rlimit / ネームスペース分離 / seccomp on Linux）
- htmx によるリアルタイム結果ポーリング
- KaTeX による数式レンダリング
- CodeMirror によるシンタックスハイライト付き提出エディタ
- PostgreSQL による提出履歴の永続化

## 技術スタック

| レイヤー | 技術 |
|---|---|
| Web フレームワーク | axum 0.8 |
| テンプレートエンジン | Tera |
| DB | PostgreSQL (sqlx 0.8) |
| サンドボックス | fork/exec + libc rlimit + seccomp (Linux) |
| フロントエンド | htmx / KaTeX / highlight.js / CodeMirror 5 |
| 開発環境 | Nix flakes + direnv |

## セットアップ

### 前提条件

- [Nix](https://nixos.org/download) (flakes 有効)
- [direnv](https://direnv.net/)

### 起動

```bash
direnv allow   # 初回のみ。PostgreSQL の初期化も行われる
dev            # PostgreSQL 起動 → マイグレーション → cargo watch -x run
```

サーバは `http://localhost:3000` で起動します。停止は `Ctrl+C`。

### 個別コマンド

```bash
pg_start    # PostgreSQL 起動
pg_stop     # PostgreSQL 停止
db-migrate  # マイグレーション実行
```

## 問題の追加

`problems/<problem_id>/` ディレクトリを作成します。

```
problems/
└── aplusb/
    ├── meta.toml        # タイトル・制限値
    ├── statement.md     # 問題文（Markdown + KaTeX）
    └── testcases/
        ├── 01.in
        └── 01.out
```

**meta.toml の形式:**

```toml
title          = "A+B Problem"
time_limit_ms  = 2000
memory_limit_kb = 262144
```

テストケースは `01.in` / `01.out`, `02.in` / `02.out` ... のように連番で追加できます（現在は最初の1件でジャッジ）。

## プロジェクト構成

```
src/
├── main.rs              # エントリポイント
├── types.rs             # Language / JudgeStatus / Submission 型定義
├── problem.rs           # 問題ファイルのロード（ディスクベース）
├── api/
│   ├── mod.rs           # AppState・ルーティング
│   └── handlers.rs      # HTML / JSON ハンドラ
├── db/
│   ├── mod.rs           # コネクションプール・マイグレーション
│   └── submission.rs    # 提出の CRUD
├── sandbox/
│   ├── mod.rs           # compile() / run_in_sandbox() の公開 API
│   ├── runner.rs        # fork + exec + wait4 による実行
│   └── seccomp.rs       # seccomp フィルタ（Linux のみ有効）
└── worker/
    └── mod.rs           # ジャッジワーカー（tokio mpsc チャンネル）
templates/
├── base.html
├── problems/
└── submissions/
problems/                # 問題ファイル（Git 管理）
migrations/              # sqlx マイグレーション
static/
└── style.css
```

## Git ワークフロー

- `master` への直接プッシュ禁止
- `dev` ブランチで作業し、PRを作成してから、マージで master へ
