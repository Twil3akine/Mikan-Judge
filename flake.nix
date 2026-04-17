{
  description = "mikan-judge dev environment";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs     = import nixpkgs { inherit system overlays; };

        # Rust: stable の最新。rust-analyzer と rust-src を同梱
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };

        # Linux のみ必要なパッケージ（macOS では無視される）
        linuxPkgs = pkgs.lib.optionals pkgs.stdenv.isLinux [
          pkgs.libseccomp          # サンドボックス用
          pkgs.libseccomp.dev
        ];

        # macOS のみ必要なパッケージ
        darwinPkgs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.Security
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain

            # C++ コンパイラ（解答コードのコンパイルに使用）
            pkgs.gcc
            pkgs.gdb          # デバッグ用

            # DB
            pkgs.postgresql_16

            # ビルドサポート
            pkgs.pkg-config
            pkgs.openssl.dev

            # 開発ツール
            pkgs.cargo-watch  # ファイル変更で自動リビルド
            pkgs.sqlx-cli     # DB マイグレーション管理
          ] ++ linuxPkgs ++ darwinPkgs;

          # PKG_CONFIG_PATH を通す（openssl, libseccomp のヘッダ検索）
          PKG_CONFIG_PATH = pkgs.lib.makeSearchPath "lib/pkgconfig" (
            [ pkgs.openssl.dev ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.libseccomp.dev ]
          );

          shellHook = ''
            echo "🍊 mikan-judge dev shell"
            echo "  Rust  : $(rustc --version)"
            echo "  g++   : $(g++ --version | head -1)"
            echo "  sqlx  : $(sqlx --version 2>/dev/null || echo 'not found')"
            echo ""
            echo "DB 起動: pg_start   DB 停止: pg_stop"
            echo "Watch : cargo watch -x run"

            # ローカル PostgreSQL を $PWD/.pg に閉じ込める
            export PGDATA="$PWD/.pg/data"
            export PGHOST="$PWD/.pg"
            export PGPORT=5432
            export PGUSER="$USER"
            export PGDATABASE="mikan_judge"
            export DATABASE_URL="postgresql://$PGUSER@localhost:$PGPORT/$PGDATABASE"

            # 初回だけ initdb
            if [ ! -d "$PGDATA" ]; then
              echo "Initializing local PostgreSQL in .pg/ ..."
              mkdir -p "$PGDATA"
              initdb -D "$PGDATA" --no-locale --encoding=UTF8 -U "$USER" -A trust \
                --listen-addresses='' 2>/dev/null
              # unix socket のみ（ポート不使用でもよいが互換性のため残す）
              echo "unix_socket_directories = '$PGHOST'" >> "$PGDATA/postgresql.conf"
            fi

            # 便利エイリアス
            alias pg_start="pg_ctl -D $PGDATA -l $PGHOST/pg.log start && \
              sleep 1 && \
              (createdb $PGDATABASE 2>/dev/null || true) && \
              echo 'PostgreSQL started: $DATABASE_URL'"
            alias pg_stop="pg_ctl -D $PGDATA stop"
          '';
        };
      }
    );
}
