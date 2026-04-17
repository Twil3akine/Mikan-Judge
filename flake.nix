{
  description = "mikan-judge dev environment";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs     = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };

        linuxPkgs = pkgs.lib.optionals pkgs.stdenv.isLinux [
          pkgs.libseccomp
          pkgs.libseccomp.dev
        ];

        # fish でも使える実行ファイルとして定義
        # env vars (PGDATA 等) は direnv 経由でエクスポートされているので参照できる
        pgStart = pkgs.writeShellScriptBin "pg_start" ''
          pg_ctl -D "$PGDATA" -l "$PGHOST/pg.log" start
          sleep 1
          createdb "$PGDATABASE" 2>/dev/null || true
          echo "PostgreSQL started: $DATABASE_URL"
        '';

        pgStop = pkgs.writeShellScriptBin "pg_stop" ''
          pg_ctl -D "$PGDATA" stop
        '';

        dev = pkgs.writeShellScriptBin "dev" ''
          pg_start
          trap 'pg_stop' EXIT INT TERM
          cargo watch -x run
        '';

      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain
            pkgs.gcc
            pkgs.gdb
            pkgs.postgresql_16
            pkgs.pkg-config
            pkgs.openssl.dev
            pkgs.cargo-watch
            pkgs.sqlx-cli
            pgStart
            pgStop
            dev
          ] ++ linuxPkgs;

          PKG_CONFIG_PATH = pkgs.lib.makeSearchPath "lib/pkgconfig" (
            [ pkgs.openssl.dev ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.libseccomp.dev ]
          );

          shellHook = ''
            echo "mikan-judge dev shell"
            echo "  Rust : $(rustc --version)"
            echo "  g++  : $(g++ --version | head -1)"
            echo ""
            echo "Start: dev"

            export PGDATA="$PWD/.pg/data"
            export PGHOST="$PWD/.pg"
            export PGPORT=5432
            export PGUSER="$USER"
            export PGDATABASE="mikan_judge"
            export DATABASE_URL="postgresql://$PGUSER@localhost:$PGPORT/$PGDATABASE"

            if [ ! -d "$PGDATA" ]; then
              echo "Initializing local PostgreSQL in .pg/ ..."
              mkdir -p "$PGHOST"
              initdb -D "$PGDATA" --no-locale --encoding=UTF8 -U "$USER" -A trust 2>/dev/null
              echo "unix_socket_directories = '$PGHOST'" >> "$PGDATA/postgresql.conf"
            fi
          '';
        };
      }
    );
}
