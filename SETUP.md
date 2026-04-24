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
dev
```

これだけです。DB の起動・サーバの起動（ファイル変更で自動再起動）を一括でやります。
Ctrl+C で止めると PostgreSQL も自動で止まります。

Hetzner に近い Linux 環境でアプリごと動かしたい場合は、こちらを使います。

```fish
dev-docker
```

これは `docker-compose.dev-docker.yml` を重ねて `judge` と `db` の両方を Docker で起動します。
ローカル macOS 実行より、実行時間・メモリ計測の確認に向いており、メモリは cgroup ベースで計測します。
起動時には judge イメージのビルド、DB の起動待ち、judge の疎通確認まで自動で行います。

個別に操作したい場合：

```fish
pg_start          # DB だけ起動
cargo watch -x run  # サーバだけ起動（ファイル変更で自動再起動）
cargo run           # サーバを1回だけ起動
pg_stop           # DB だけ停止
judge-smoke-test  # judge の主要言語スモークテスト
```

### judge のスモークテスト

言語追加や sandbox 調整のあとに、judge が壊れていないかをまとめて確認するためのコマンドです。

```fish
judge-smoke-test
```

内部では `cargo test smoke_aplusb_across_languages -- --nocapture` を実行します。
`problems/aplusb/` の先頭ケースを使って、以下を確認します。

- `cpp`, `rust`, `python`, `pypy`, `java`, `go` は AC 相当
- `text` は WA 相当

内部では、各言語向けに最小の `A+B` プログラムをその場で作って、
`compile()` と `run_in_sandbox()` を通し、1 ケースだけ実行しています。
なので、少なくとも以下の健全性確認にはなります。

- その言語のツールチェーンが見つかる
- judge 側のコンパイル手順が壊れていない
- sandbox 内で実行できる
- 出力比較まで一通り通る

ただし、これはスモークテストなので、時間制限ぎりぎりのケースや大きいメモリ使用量、
複数ケース連続実行、Linux 本番相当の厳密な実行時間までは保証しません。

成功時は末尾が次のようになります。

```text
test sandbox::tests::smoke_aplusb_across_languages ... ok
test result: ok. 1 passed; 0 failed;
```

失敗した場合は、どの言語で `compile` または `run` が失敗したかがそのまま表示されます。

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
