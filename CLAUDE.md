# CLAUDE.md — MikanJudge Agent Instructions

## Project Overview

Rust製の競技プログラミング用オンラインジャッジ。axum + Tera SSR + PostgreSQL (sqlx) + htmx。

## Architecture

```
src/
├── main.rs          # AppState (db pool, tera, problems_dir), routing
├── types.rs         # Language / JudgeStatus / Submission 型定義
├── problem.rs       # 問題ファイルのロード (load_one / load_all)
├── api/
│   ├── mod.rs       # AppState・ルーティング定義
│   └── handlers.rs  # HTML / JSON ハンドラ
├── db/
│   ├── mod.rs       # コネクションプール・マイグレーション実行
│   └── submission.rs# 提出の CRUD (insert / get / update_result / list_recent)
├── sandbox/
│   ├── mod.rs       # compile() / run_in_sandbox() の公開 API
│   ├── runner.rs    # fork + exec + wait4 による実行（ブロッキング）
│   └── seccomp.rs   # seccomp フィルタ（Linux のみ有効、cfg(target_os="linux")）
└── worker/
    └── mod.rs       # ジャッジワーカー（tokio mpsc チャンネル）
```

## Key Patterns

### Language enum (`src/types.rs`)
- バリアント: `Cpp`, `Rust`, `Python`, `PyPy`
- `is_interpreted()` → コンパイル不要かどうか
- `interpreter()` → "python3" / "pypy3"
- `display_name()` → "C++17" / "Rust" / "Python3 (CPython)" / "Python3 (PyPy)"
- `to_db()` / `from_db()` → PostgreSQL の文字列表現
- `extension()` → ソースファイル拡張子

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
- CE時: `update_result(..., stderr=Some(compile_error_message))`
- コンパイル警告+実行stderr: `[Compile warnings]\n<warnings>\n\n<stderr>` の形式で結合

### Templates (Tera)
- `templates/base.html`: ナビ・KaTeX・highlight.js・htmx を含む共通レイアウト
- `{% block scripts %}`: ページ固有JS（CodeMirror等）を追加する場所
- highlight.js: `.statement` クラスの中の `<code>` はスキップ（問題文コードブロックは黒文字）
- htmx ポーリング: `<div hx-get="..." hx-trigger="every 1s" hx-swap="outerHTML">` で結果を1秒ごと更新
- `templates/submissions/poll.html`: htmx のスワップターゲット（verdict部分のみ）

### Problem files
```
problems/<id>/
├── meta.toml        # title, time_limit_ms, memory_limit_kb
├── statement.md     # Markdown + KaTeX
└── testcases/
    ├── 01.in
    └── 01.out
```
- `problem::load_one(dir, id)` / `problem::load_all(dir)` でロード
- Markdown→HTML は pulldown-cmark

## Development Environment

- Nix flakes + direnv
- `dev` コマンド: `pg_start → db-migrate → cargo watch -x run`
- `pg_start` / `pg_stop` / `db-migrate` は `flake.nix` の `writeShellScriptBin` で定義
- Python: `pkgs.python3` (CPython), `pkgs.pypy3` (PyPy) が buildInputs に含まれる
- macOS では dyld/ObjC ランタイムの起動コストにより実行時間が大きく見える（C++でも~50ms）。正確な計測はLinux（Docker等）が必要。

## Database

- PostgreSQL, sqlx 0.8, migrations は `migrations/` ディレクトリ
- `db-migrate` で `sqlx migrate run`
- 主なテーブル: `submissions` (id UUID, problem_id, language, source_code, status, stdout, stderr, time_used_ms, memory_used_kb, created_at)

## Git Workflow

- `master` への直接プッシュ禁止
- `dev` ブランチで作業
- PR を作成して、master にマージする
- コミットは適切な粒度で（機能単位・修正単位）
- コミットメッセージは `feat:` / `fix:` / `chore:` プレフィックスを使う

## macOS 固有の注意

- seccomp は Linux のみ (`#[cfg(target_os = "linux")]`)
- `RLIMIT_AS` はインタープリタ言語に対しては設定しない
- `execvp` を使うことで PATH 経由でインタープリタを解決
- `resolve_interpreter()` で `which` コマンドを使い絶対パスを取得（macOS の `/usr/bin/python3` スタブ回避）
