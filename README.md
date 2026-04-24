# MikanJudge 🍊

和歌山大学競技プログラミングサークル **WCPC** による、Rust 製オンラインジャッジシステム

## 機能

- C++17 / Rust / Python3 (CPython) / Python3 (PyPy) での提出・ジャッジ
- 全テストケースをジャッジし、ケースごとの AC/WA/TLE/RE を表示
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
| DB | PostgreSQL 16（Docker コンテナ） |
| 認証 | argon2 0.5 + tower-sessions 0.15 |
| サンドボックス | fork/exec + libc rlimit + seccomp (Linux) |
| フロントエンド | htmx / KaTeX / highlight.js / CodeMirror 5 |
| 開発環境 | Nix flakes + direnv + Docker |

## セットアップ（開発）

### 前提条件

- [Nix](https://nixos.org/download)（flakes 有効）
- [direnv](https://direnv.net/)
- [OrbStack](https://orbstack.dev/) または Docker Desktop（PostgreSQL の起動に使用）

### 起動

```bash
direnv allow   # 初回のみ
dev            # Docker で PostgreSQL 起動 → マイグレーション → cargo watch
```

サーバは `http://localhost:3000` で起動します。停止は `Ctrl+C`（PostgreSQL コンテナも自動停止）。

Linux 本番に近い環境で judge まで含めて動かしたい場合は、こちらを使います。

```bash
dev-docker     # Docker で judge + PostgreSQL を起動
```

こちらは `docker compose up --build` でアプリ本体も Linux コンテナ内で実行します。
実行時間・メモリの確認は `dev` より `dev-docker` の方が本番に近いです。
`dev-docker` は judge イメージのビルド、DB の起動待ち、judge の疎通確認まで行ったうえで
judge ログを追尾します。

### 個別コマンド

```bash
db-migrate     # マイグレーション実行（DB 起動中に使用）
```

### データの扱い

PostgreSQL のデータは Docker ボリューム `mikan-judge_postgres_data` に保存されます。

| 操作 | データ |
|---|---|
| `dev` を止める（Ctrl+C） | **残る** |
| `docker compose down` | **残る** |
| `docker compose down -v` | **消える**（明示的なボリューム削除） |

## デプロイ（本番 / Hetzner 等）

### 初回

```bash
git clone <repo>
cd mikan-judge
cp .env.example .env
# .env の POSTGRES_PASSWORD を変更する
docker compose up --build -d
```

### コード更新時

```bash
git pull
docker compose up --build -d
```

ボリューム（`-v` なし）はそのままなので、**提出データ・ユーザデータは保持されます。**

### ⚠ データが消えるコマンド

```bash
docker compose down -v   # 実行しないこと（ボリュームごと削除）
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
├── index.html           # ランディングページ（開催中コンテストのみ・About）
├── auth/
│   ├── login.html
│   └── register.html
├── contests/
│   ├── list.html        # コンテスト一覧（全ステータス）
│   ├── problems/        # コンテスト内問題一覧・詳細
│   ├── submissions/     # コンテスト内提出一覧・詳細
│   └── standings.html   # 順位表
├── problems/            # コンテスト外問題ページ（後方互換）
└── submissions/         # コンテスト外提出ページ（後方互換）
problems/                # 問題ファイル（Git 管理）
migrations/              # sqlx マイグレーション
static/
└── style.css
Dockerfile               # multi-stage ビルド（本番イメージ）
docker-compose.yml       # judge + PostgreSQL
.env.example             # 環境変数テンプレート
```

## URL 構成

| パス | 説明 |
|---|---|
| `/` | ランディングページ（開催中コンテストのみ・About） |
| `/contests` | コンテスト一覧（開催中・予定・過去） |
| `/contests/:id/problems` | コンテスト内問題一覧 |
| `/contests/:id/problems/:pid` | 問題詳細・提出フォーム |
| `/contests/:id/submissions` | 提出一覧（ページネーション付き） |
| `/contests/:id/submissions/:sid` | 提出詳細 |
| `/contests/:id/standings` | 順位表（ページネーション付き） |

## Git ワークフロー

- `master` への直接コミット・プッシュ禁止
- 作業開始時に必ず機能ブランチ（`feat/xxx`）を切る
- PR を作成して master にマージ
- コミットは機能単位で細かく分ける（handler / template / CSS は別コミット）
- コミットメッセージは `feat:` / `fix:` / `chore:` / `style:` / `docs:` プレフィックスを使う
