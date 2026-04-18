# CLAUDE.md — MikanJudge Agent Instructions

## Project Overview

和歌山大学競技プログラミングサークル **WCPC** 向けの Rust 製オンラインジャッジ。axum + Tera SSR + PostgreSQL (sqlx) + htmx。

## Architecture

```
src/
├── main.rs              # AppState (db pool, tera, problems_dir), routing
├── types.rs             # Language / JudgeStatus / Submission / Contest / ContestProblem 型定義
├── problem.rs           # 問題ファイルのロード (load_one / load_all)
├── session_store.rs     # PgSessionStore: tower-sessions SessionStore の PostgreSQL 実装
├── api/
│   ├── mod.rs           # AppState・ルーティング定義・セッションレイヤー設定
│   └── handlers.rs      # HTML / JSON ハンドラ
├── db/
│   ├── mod.rs           # コネクションプール・マイグレーション実行
│   ├── contest.rs       # コンテスト CRUD (list_all / list_grouped / get_by_id / problems_for_contest)
│   ├── submission.rs    # 提出 CRUD (insert / get / update_result / list_for_contest / first_acs_for_contest)
│   └── user.rs          # ユーザ CRUD (insert / find_by_username / find_by_id)
├── sandbox/
│   ├── mod.rs           # compile() / run_in_sandbox() の公開 API
│   ├── runner.rs        # fork + exec + wait4 による実行（ブロッキング）
│   └── seccomp.rs       # seccomp フィルタ（Linux のみ有効、cfg(target_os="linux")）
└── worker/
    └── mod.rs           # ジャッジワーカー（tokio mpsc チャンネル）
```

## URL Structure

コンテスト中心の URL 体系を採用。

| ルート | ハンドラ | 説明 |
|---|---|---|
| `GET /` | `index` | ランディングページ（開催中コンテストのみ・About） |
| `GET /contests` | `contests_index` | コンテスト一覧（全ステータス） |
| `GET /contests/:id` | `contest_detail` | → `/contests/:id/problems` にリダイレクト |
| `GET /contests/:id/problems` | `contest_problems_index` | コンテスト内問題一覧 |
| `GET /contests/:id/problems/:pid` | `contest_problem_detail` | 問題詳細・提出フォーム |
| `POST /contests/:id/problems/:pid/submit` | `contest_problem_submit` | 提出（クールダウンチェック付き） |
| `GET /contests/:id/submissions` | `contest_submissions_index` | 提出一覧（20件/ページ） |
| `GET /contests/:id/submissions/:sid` | `contest_submission_detail` | 提出詳細 |
| `GET /contests/:id/standings` | `contest_standings` | 順位表（20件/ページ） |

旧 `/problems/*`, `/submissions/*` ルートも後方互換のため残存（contest_id=None）。

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
- 提出間隔制限: セッションに `last_submit_at: i64`（Unix ミリ秒）を保存し、5 秒以内の再提出を拒否。違反時は `?cooldown_remaining_ms=N` クエリパラメータでリダイレクトし、テンプレート側でカウントダウントーストを表示

### Contest (`src/types.rs`, `src/db/contest.rs`)
- `Contest { id, title, description, start_time, end_time }` — chrono `DateTime<Utc>`
- `Contest::status() -> ContestStatus` — Ongoing / Upcoming / Past を now と比較して返す
- `ContestStatus::label()` / `badge_class()` → テンプレート表示用
- `db::contest::list_grouped(pool)` → `ContestLists { ongoing, upcoming, past }` を返す
- `db::contest::problems_for_contest(pool, contest_id)` → `Vec<ContestProblem>` を返す
- コンテストと問題の紐付けは `contest_problems` テーブル（label="A"/"B"/...）

### Standings (`src/api/handlers.rs`)
- `db::submission::first_acs_for_contest(pool, contest_id)` → ユーザ×問題ごとの初回 AC 時刻
- ハンドラで集計: ユーザごとの total_score・last_ac（DateTime）を計算
- ソート: total_score DESC → last_ac_raw ASC（None は最後）
- 表示用時刻はすべて **コンテスト開始からの経過時間**（`H:MM:SS` 形式）に変換
- 同点判定は `elapsed_from_start` 文字列で比較
- `build_pagination(current, total) -> Vec<i64>`: 0 が省略記号（…）、それ以外はページ番号。7ページ以下は全表示、それ以上はウィンドウ±2で省略

### Submission Pagination
- `db::submission::list_for_contest(pool, contest_id, page, per_page)` → 20件/ページ
- `db::submission::count_for_contest(pool, contest_id)` → 総件数
- `build_pagination` ヘルパーを共用

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
- `templates/base.html`: ナビ（`contest_id` が Some のときコンテスト内リンク、None のときコンテスト一覧のみ）・KaTeX・highlight.js・htmx を含む共通レイアウト
- 全ハンドラは `contest_id: Option<String>` をコンテキストに挿入する
- `templates/index.html`: ランディングページ（開催中コンテストのみ・About・連絡先）
- `templates/contests/list.html`: コンテスト一覧（Ongoing / Upcoming / Past 全ステータス）
- `templates/auth/`: login.html / register.html
- `templates/contests/problems/`: index.html / detail.html（提出フォーム・クールダウントースト）
- `templates/contests/submissions/`: index.html（ページネーション）/ detail.html
- `templates/contests/standings.html`: 順位表（ページネーション）
- `{% block scripts %}`: ページ固有JS（CodeMirror等）を追加する場所
- highlight.js: `.statement` クラスの中の `<code>` はスキップ（問題文コードブロックは黒文字）
- htmx ポーリング: `<div hx-get="..." hx-trigger="every 1s" hx-swap="outerHTML">` で結果を1秒ごと更新
- `templates/submissions/poll.html`: htmx のスワップターゲット（verdict部分のみ、コンテスト内外共用）

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
| `submissions` | id UUID, user_id UUID→users, contest_id TEXT→contests, problem_id, language, source_code, status, stdout, stderr, time_used_ms, memory_used_kb, testcase_results TEXT(JSON配列), created_at |
| `tower_sessions` | id TEXT, data TEXT(JSON), expiry_unix BIGINT — セッション永続化 |
| `contests` | id TEXT, title, description, start_time TIMESTAMPTZ, end_time TIMESTAMPTZ, created_at |
| `contest_problems` | contest_id→contests, problem_id, display_order INT, label TEXT ("A"/"B"/...) |

## Git Workflow

- `master` への直接コミット・プッシュは禁止（「コミットして」と言われても必ずブランチを切ること）
- 作業開始時に必ず機能ブランチ（`feat/xxx`）を切る
- PR を作成して master にマージする
- コミットは細かく分ける（migration / DB / handler / template / CSS は別コミット）
- コミットメッセージは `feat:` / `fix:` / `chore:` / `style:` / `docs:` プレフィックスを使う

## macOS 固有の注意

- seccomp は Linux のみ (`#[cfg(target_os = "linux")]`)
- `RLIMIT_AS` はインタープリタ言語に対しては設定しない
- `execvp` を使うことで PATH 経由でインタープリタを解決
- `resolve_interpreter()` で `which` コマンドを使い絶対パスを取得（macOS の `/usr/bin/python3` スタブ回避）
- macOS の `ru_maxrss` は bytes 単位（Linux は KB 単位）でかつ dyld/ObjC ランタイム分が加算されるため、メモリ計測値が実際より大きく見える
