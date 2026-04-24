# syntax=docker/dockerfile:1

# ============================================================
# Stage 1: builder
#   Nix の devShell と同等の依存を apt で揃えて cargo build する。
#   rust-overlay が管理するツールチェーンをそのままコピーすることで
#   Runtime stage でも同一バージョンの rustc を使える。
# ============================================================
FROM rust:1.94-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    libseccomp-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# ── 依存クレートのキャッシュ層 ──────────────────────────────────
# Cargo.toml / Cargo.lock だけ先にコピーしてダミー main でビルドし、
# 依存クレートのコンパイル結果をレイヤーに残す。
# ソースを変更しても依存クレートの再コンパイルが不要になる。
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs && \
    cargo build --release --locked && \
    rm -f target/release/mikan-judge target/release/deps/mikan_judge*

# ── 本体のビルド ────────────────────────────────────────────────
COPY src        ./src
COPY migrations ./migrations
RUN cargo build --release --locked

# ============================================================
# Stage 2: runtime
#   judge が提出コードをコンパイル・実行するために必要なツールを
#   ランタイムイメージに含める。
#   rustc は builder の rustup ごとコピーすることでバージョンを固定。
# ============================================================
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libseccomp2 \
    # C++ 提出のコンパイル
    g++ \
    # Python 提出の実行
    python3 \
    # PyPy 提出の実行
    pypy3 \
    # resolve_interpreter() が which コマンドを使用する
    debianutils \
    && rm -rf /var/lib/apt/lists/*

# Rust 提出のコンパイル用: builder の rustup / cargo をそのままコピー
# （apt の rustc はバージョンが古いため builder のものを流用する）
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:$PATH"
COPY --from=builder /usr/local/rustup /usr/local/rustup
COPY --from=builder /usr/local/cargo  /usr/local/cargo

# アプリ本体
COPY --from=builder /build/target/release/mikan-judge /usr/local/bin/mikan-judge

# テンプレート・静的ファイル（Tera が起動時に読み込む）
# problems/ はコンテンツなので docker-compose でボリュームマウントする
COPY templates /app/templates
COPY static    /app/static

WORKDIR /app

EXPOSE 3000
ENV RUST_LOG=info

CMD ["/usr/local/bin/mikan-judge"]
