# CLAUDE.md — MikanJudge Agent Instructions

## Project Overview

Rust製の競技プログラミング用オンラインジャッジ。axum + Tera SSR + PostgreSQL (sqlx) + htmx。

## Architecture

```
src/
├── main.rs              # AppState (db pool, tera, problems_dir), routing
├── types.rs             # Language / JudgeStatus / Submission / Contest 型定義
├── problem.rs           # 問題ファイルのロード (load_one / load_all)
├── session_store.rs     # PgSessionStore: tower-sessions SessionStore の PostgreSQL 実装
├── api/
│   ├── mod.rs           # AppState・ルーティング定義・セッションレイヤー設定
│   └── handlers.rs      # HTML / JSON ハンドラ
├── db/
│   ├── mod.rs           # コネクションプール・マイグレーション実行
│   ├── contest.rs       # コンテスト CRUD (list_all / list_grouped / problem_ids)
│   ├── submission.rs    # 提出 CRUD (insert / get / update_result / list_recent)
│   └── user.rs          # ユーザ CRUD (insert / find_by_username / find_by_id)
├── sandbox/
│   ├── mod.rs           # compile() / run_in_sandbox() の公開 API
│   ├── runner.rs        # fork + exec + wait4 による実行（ブロッキング）
│   └── seccomp.rs       # seccomp フィルタ（Linux のみ有効、cfg(target_os="linux")）
└── worker/
    └── mod.rs           # ジャッジワーカー（tokio mpsc チャンネル）
```

## Key Patterns

### Language enum (`src/types.rs`)
- バリアント: `Cpp`, `Rust`, `Python`, `PyPy`
- `is_interpreted()` → コンパイル不要かどうか
- `interpreter()` → "python3" / "pypy3"
- `display_name()` → "C++17" / "Rust" / "Python3 (CPython)" / "Python3 (PyPy)"
- `to_db()` / `from_db()` → PostgreSQL の文字列表現
- `extension()` → ソースファイル拡張子

### Auth & Session
- パスワードは **argon2 0.5** でハッシュ化し `users.password_hash` に保存
- セッションは **tower-sessions 0.15** + 自前の `PgSessionStore`（`src/session_store.rs`）
  - `tower-sessions-sqlx-store` は tower-sessions 0.15 と `tower_sessions_core` のバージョンが食い違うため使用不可
  - セッションデータは `tower_sessions` テーブル（id TEXT, data TEXT JSON, expiry_unix BIGINT）に保存
  - `store.migrate()` はアプリ起動時に `create_router()` 内で呼ばれる
- セッションに保存するのは `user_id: Uuid` のみ。各リクエストで `find_by_id` でユーザ名を引く
- `current_username(session, pool) -> Option<String>` ヘルパーが全 HTML ハンドラで使われる
- 未ログインでの提出は許可（`user_id = NULL`）

### Contest (`src/types.rs`, `src/db/contest.rs`)
- `Contest { id, title, description, start_time, end_time }` — chrono `DateTime<Utc>`
- `Contest::status() -> ContestStatus` — Ongoing / Upcoming / Past を now と比較して返す
- `ContestStatus::label()` / `badge_class()` → テンプレート表示用
- `db::contest::list_grouped(pool)` → `ContestLists { ongoing, upcoming, past }` を返す
- コンテストと問題の紐付けは `contest_problems` テーブル（label="A"/"B"/...）

### Sandbox (`src/sandbox/mod.rs`, `runner.rs`)
- `compile(language, source, work_dir) -> Result<CompileOutput>`
  - インタープリタ言語: `py_compile` で構文チェックのみ、実際にはコンパイルしない
  - `resolve_interpreter(name)` で `which python3/pypy3` を呼び絶対パスを取得
- `CompileOutput { executable, run_args, error, warnings }`
  - インタープリタ言語: `executable = /path/to/python3`, `run_args = [source_path]`
- `run_in_sandbox(executable, run_args, stdin, config) -> Result<RunResult>`
- `RunResult { stdout, stderr, status, time_used, cpu_time_used }`
  - `cpu_time_used`: `wait4(pid, &rusage)` から ru_utime + ru_stime で計測（RUSAGE_CHILDREN は累積するため使わない）
- `SandboxConfig`
  - `vm_limit_bytes: Option<u64>` — インタープリタ言語は `None`（Python は起動時に大量のVASを使う）
  - Linux のみ: `unshare(CLONE_NEWNET)` + seccomp フィルタ

### Worker (`src/worker/mod.rs`)
- tokio mpsc チャンネルでジャッジジョブをキュー管理
- `JudgeJob.testcases: Vec<(String, String)>` — 全テストケース（input, expected）
- 全ケースを順にジャッジし `tc_verdicts: Vec<String>` に "AC"/"WA"/"TLE"/"MLE"/"RE" を収集
- 最初の非 AC で残りは "--" で埋める
- CE時: `update_result(..., stderr=Some(compile_error_message))`
- コンパイル警告+実行stderr: `[Compile warnings]\n<warnings>\n\n<stderr>` の形式で結合

### Templates (Tera)
- `templates/base.html`: ナビ（ログイン状態に応じてログアウト/ログインリンク）・KaTeX・highlight.js・htmx を含む共通レイアウト
- `templates/index.html`: ランディングページ（コンテスト一覧 Ongoing/Upcoming/Past）
- `templates/auth/`: login.html / register.html
- `{% block scripts %}`: ページ固有JS（CodeMirror等）を追加する場所
- highlight.js: `.statement` クラスの中の `<code>` はスキップ（問題文コードブロックは黒文字）
- htmx ポーリング: `<div hx-get="..." hx-trigger="every 1s" hx-swap="outerHTML">` で結果を1秒ごと更新
- `templates/submissions/poll.html`: htmx のスワップターゲット（verdict部分のみ）

### Problem files
```
problems/<id>/
├── meta.toml        # title, time_limit_ms, memory_limit_kb, score
├── statement.md     # Markdown + KaTeX
└── testcases/
    ├── 01.in / 01.out
    └── 02.in / 02.out  (連番で複数可)
```
- `problem::load_one(dir, id)` / `problem::load_all(dir)` でロード
- Markdown→HTML は pulldown-cmark
- `score` フィールドはデフォルト 100

## Development Environment

- Nix flakes + direnv
- `dev` コマンド: `pg_start → db-migrate → cargo watch -x run`
- `pg_start` / `pg_stop` / `db-migrate` は `flake.nix` の `writeShellScriptBin` で定義
- Python: `pkgs.python3` (CPython), `pkgs.pypy3` (PyPy) が buildInputs に含まれる
- macOS では dyld/ObjC ランタイムの起動コストにより実行時間が大きく見える（C++でも~50ms）。正確な計測はLinux（Docker等）が必要。

## Database

- PostgreSQL, sqlx 0.8, migrations は `migrations/` ディレクトリ
- `db-migrate` で `sqlx migrate run`

### テーブル一覧

| テーブル | 概要 |
|---|---|
| `users` | id UUID, username TEXT UNIQUE, password_hash TEXT, created_at |
| `submissions` | id UUID, user_id UUID→users, problem_id, language, source_code, status, stdout, stderr, time_used_ms, memory_used_kb, testcase_results TEXT(JSON配列), created_at |
| `tower_sessions` | id TEXT, data TEXT(JSON), expiry_unix BIGINT — セッション永続化 |
| `contests` | id TEXT, title, description, start_time TIMESTAMPTZ, end_time TIMESTAMPTZ, created_at |
| `contest_problems` | contest_id→contests, problem_id, display_order INT, label TEXT ("A"/"B"/...) |

## Git Workflow

- `master` への直接プッシュ禁止
- 機能ブランチ（`feat/xxx`）で作業
- PR を作成して master にマージする
- コミットは適切な粒度で（機能単位・修正単位）
- コミットメッセージは `feat:` / `fix:` / `chore:` / `style:` / `docs:` プレフィックスを使う

## macOS 固有の注意

- seccomp は Linux のみ (`#[cfg(target_os = "linux")]`)
- `RLIMIT_AS` はインタープリタ言語に対しては設定しない
- `execvp` を使うことで PATH 経由でインタープリタを解決
- `resolve_interpreter()` で `which` コマンドを使い絶対パスを取得（macOS の `/usr/bin/python3` スタブ回避）
- macOS の `ru_maxrss` は bytes 単位（Linux は KB 単位）でかつ dyld/ObjC ランタイム分が加算されるため、メモリ計測値が実際より大きく見える
