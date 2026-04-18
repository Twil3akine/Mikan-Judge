# MikanJudge 🍊

和歌山大学競技プログラミングサークル **WCPC** による、Rust 製オンラインジャッジシステム

## 機能

- C++17 / Rust / Python3 (CPython) / Python3 (PyPy) での提出・ジャッジ
- 全テストケースをジャッジし、ケースごとの AC/WA/TLE/MLE/RE を表示
- fork + exec サンドボックス（rlimit / ネームスペース分離 / seccomp on Linux）
- htmx によるリアルタイム結果ポーリング
- KaTeX による数式レンダリング
- CodeMirror によるシンタックスハイライト付き提出エディタ
- ユーザ登録・ログイン（argon2 パスワードハッシュ）
- PostgreSQL バックドセッション（サーバ再起動後も維持）
- コンテスト管理（開催中 / 予定 / 終了）
- 順位表（得点 DESC → コンテスト開始からの経過時間 ASC）
- 提出一覧・順位表のページネーション（20件/ページ）
- 提出間隔制限（5 秒クールダウン）

## 技術スタック

| レイヤー | 技術 |
|---|---|
| Web フレームワーク | axum 0.8 |
| テンプレートエンジン | Tera |
| DB | PostgreSQL (sqlx 0.8) |
| 認証 | argon2 0.5 + tower-sessions 0.15 |
| サンドボックス | fork/exec + libc rlimit + seccomp (Linux) |
| フロントエンド | htmx / KaTeX / highlight.js / CodeMirror 5 |
| 開発環境 | Nix flakes + direnv |

## セットアップ

### 前提条件

- [Nix](https://nixos.org/download)（flakes 有効）
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
    ├── meta.toml        # タイトル・制限値・配点
    ├── statement.md     # 問題文（Markdown + KaTeX）
    └── testcases/
        ├── 01.in
        ├── 01.out
        ├── 02.in
        └── 02.out
```

**meta.toml の形式:**

```toml
title           = "A+B Problem"
time_limit_ms   = 2000
memory_limit_kb = 131072   # 128 MiB
score           = 100
```

テストケースは `01.in` / `01.out`, `02.in` / `02.out` ... のように連番で追加します。全ケースがジャッジされます。

## コンテストの追加

`contests` テーブルに直接 INSERT します。

```sql
INSERT INTO contests (id, title, description, start_time, end_time)
VALUES ('abc001', 'MikanJudge Contest 001', '',
        '2025-05-01 21:00:00+09', '2025-05-01 23:00:00+09');

INSERT INTO contest_problems (contest_id, problem_id, display_order, label)
VALUES ('abc001', 'aplusb', 1, 'A');
```

## プロジェクト構成

```
src/
├── main.rs              # エントリポイント
├── types.rs             # Language / JudgeStatus / Submission / Contest 型定義
├── problem.rs           # 問題ファイルのロード（ディスクベース）
├── session_store.rs     # PostgreSQL バックドセッションストア
├── api/
│   ├── mod.rs           # AppState・ルーティング・セッションレイヤー
│   └── handlers.rs      # HTML / JSON ハンドラ
├── db/
│   ├── mod.rs           # コネクションプール・マイグレーション
│   ├── contest.rs       # コンテストの CRUD
│   ├── submission.rs    # 提出の CRUD
│   └── user.rs          # ユーザの CRUD
├── sandbox/
│   ├── mod.rs           # compile() / run_in_sandbox() の公開 API
│   ├── runner.rs        # fork + exec + wait4 による実行
│   └── seccomp.rs       # seccomp フィルタ（Linux のみ有効）
└── worker/
    └── mod.rs           # ジャッジワーカー（tokio mpsc チャンネル）
templates/
├── base.html
├── index.html           # ランディングページ（コンテスト一覧）
├── auth/
│   ├── login.html
│   └── register.html
├── contests/
│   ├── problems/        # コンテスト内問題一覧・詳細
│   ├── submissions/     # コンテスト内提出一覧・詳細
│   └── standings.html   # 順位表
├── problems/            # コンテスト外問題ページ（後方互換）
└── submissions/         # コンテスト外提出ページ（後方互換）
problems/                # 問題ファイル（Git 管理）
migrations/              # sqlx マイグレーション
static/
└── style.css
```

## URL 構成

| パス | 説明 |
|---|---|
| `/` | コンテスト一覧 |
| `/contests/:id/problems` | コンテスト内問題一覧 |
| `/contests/:id/problems/:pid` | 問題詳細・提出フォーム |
| `/contests/:id/submissions` | 提出一覧（ページネーション付き） |
| `/contests/:id/submissions/:sid` | 提出詳細 |
| `/contests/:id/standings` | 順位表（ページネーション付き） |

## Git ワークフロー

- `master` への直接プッシュ禁止
- 機能ブランチ（`feat/xxx`）で作業し、PR を作成してマージ
- コミットは機能単位で細かく分ける
- コミットメッセージは `feat:` / `fix:` / `chore:` / `style:` / `docs:` プレフィックスを使う
