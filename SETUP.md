# mikan-judge 開発環境セットアップガイド

## この環境で何が起きているか

このプロジェクトは **Nix** を使って開発環境を管理しています。
`nix develop` を実行すると、`flake.nix` に書かれた

- Rust（コンパイラ + rust-analyzer + clippy + rustfmt）
- g++（提出された C++ コードのコンパイルに使う）
- PostgreSQL 16
- sqlx-cli（DB マイグレーション管理）
- cargo-watch（ファイル変更で自動リビルド）

が**このプロジェクトの中だけ**に入ります。グローバルな環境は汚れません。

## あなたの環境で必要なもの（確認済み ✅）

| ツール | 状態 |
|--------|------|
| Nix (Determinate) | ✅ インストール済み |
| flakes 有効化 | ✅ 設定済み |
| direnv | ✅ インストール済み |
| fish との連携 | ✅ 設定済み |

**追加作業は何もありません。** 以下の手順ですぐ使えます。

---

## 初回セットアップ（最初の1回だけ）

```fish
cd ~/mein/coding-space/mikan-judge
direnv allow
```

これだけです。`direnv allow` を実行すると：

1. `flake.nix` を読み込んで必要なパッケージを Nix がダウンロード・ビルド
2. 自動的に dev シェルに入る
3. `~/mein/coding-space/mikan-judge` に入るたびに自動で環境が有効化される

> **初回は数分かかります**（Rust や PostgreSQL のダウンロードのため）。
> 2回目以降はキャッシュが効くので一瞬です。

---

## 日常の使い方

### プロジェクトディレクトリに入るだけで自動で環境が有効化される

```fish
cd ~/mein/coding-space/mikan-judge
# ↑ これだけで Rust, g++, psql などが使える状態になる
```

ターミナルの表示が変わって `(nix:mikan-judge)` のようなプレフィックスが出ます。

### サーバを起動する

```fish
# DB を起動（初回は自動で初期化される）
pg_start

# サーバを起動（ファイルを変えると自動で再起動）
cargo watch -x run

# または普通に起動
cargo run
```

### サーバを止める

```fish
# Ctrl+C でサーバを止める
# DB も止める場合
pg_stop
```

### 動作確認（curl）

```fish
# ヘルスチェック
curl http://localhost:3000/health

# コードを提出
curl -X POST http://localhost:3000/submit \
  -H 'Content-Type: application/json' \
  -d '{
    "source_code": "#include<iostream>\nint main(){int a,b;std::cin>>a>>b;std::cout<<a+b;}\n",
    "language": "cpp",
    "problem_id": "aplusb",
    "stdin": "3 5",
    "expected_output": "8",
    "time_limit_ms": 2000,
    "memory_limit_kb": 262144
  }'

# 結果を確認（上のレスポンスの id を使う）
curl http://localhost:3000/result/<ここにidを貼る>
```

---

## PostgreSQL について

ローカルの PostgreSQL は `.pg/` ディレクトリに閉じ込めてあります。
システムの PostgreSQL とは完全に別物で、このプロジェクト専用です。

```fish
pg_start    # 起動 & データベース作成（初回のみ作成）
pg_stop     # 停止
psql        # DB に接続して SQL を直接打てる
```

接続先は自動で設定されています：
```
DATABASE_URL=postgresql://<あなたのユーザー名>@localhost:5432/mikan_judge
```

---

## よくあるトラブル

### `direnv allow` したのに何も変わらない

```fish
# direnv の状態を確認
direnv status

# 手動で再読み込み
direnv reload
```

### `pg_start` してもエラーが出る

```fish
# ログを確認
cat .pg/pg.log

# .pg を消して初期化しなおす（データが消えるので注意）
pg_stop
rm -rf .pg
pg_start
```

### `cargo run` でコンパイルエラーが出る

```fish
# まず cargo check で確認
cargo check
```

### 環境から抜けたい

```fish
# 別ディレクトリに移動するだけで自動的に無効化される
cd ~
```

---

## macOS での注意点

このプロジェクトのサンドボックス機能（seccomp・ネットワーク分離）は **Linux 専用** です。
macOS では自動的に無効化されるので、ローカルでは動作確認はできますが、
サンドボックスはほぼ効いていない状態です。

**本番デプロイは Hetzner VPS（Linux）で行います。**

---

## ファイル構成（参考）

```
mikan-judge/
├── flake.nix       ← Nix 環境定義（触る必要はほぼなし）
├── .envrc          ← direnv の設定（use flake と書いてあるだけ）
├── Cargo.toml      ← Rust の依存クレート
├── src/
│   ├── main.rs
│   ├── types.rs
│   ├── sandbox/    ← サンドボックス実装
│   ├── worker/     ← ジョブキュー
│   └── api/        ← HTTP API (axum)
└── .pg/            ← ローカル PostgreSQL データ（gitignore済み）
```
