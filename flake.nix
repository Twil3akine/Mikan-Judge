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

        rustToolchain = pkgs.rust-bin.stable."1.94.1".default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };

        linuxPkgs = pkgs.lib.optionals pkgs.stdenv.isLinux [
          pkgs.libseccomp
          pkgs.libseccomp.dev
        ];

        # ── アプリ本体のデリバティブ ──────────────────────────────
        # nix build .#default でバイナリをビルドする。
        # nix build .#dockerImage（Linux のみ）で Docker イメージを生成する。
        rustPkg = pkgs.rustPlatform.buildRustPackage {
          pname   = "mikan-judge";
          version = "0.1.0";
          src     = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.libseccomp ];

          PKG_CONFIG_PATH = pkgs.lib.makeSearchPath "lib/pkgconfig" (
            [ pkgs.openssl.dev ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.libseccomp.dev ]
          );
        };

        # テンプレート・静的ファイルを /app に配置するデリバティブ
        # buildLayeredImage の contents に含めることでイメージに焼き込む
        appFiles = pkgs.stdenv.mkDerivation {
          name  = "mikan-judge-app-files";
          src   = ./.;
          phases = [ "installPhase" ];
          installPhase = ''
            mkdir -p $out/app/templates $out/app/static $out/app/problems
            cp -r $src/templates/. $out/app/templates/
            cp -r $src/static/.   $out/app/static/
          '';
        };

        # マイグレーションを実行する（DB が起動している必要がある）
        dbMigrate = pkgs.writeShellScriptBin "db-migrate" ''
          sqlx migrate run --database-url "$DATABASE_URL"
        '';

        devDocker = pkgs.writeShellScriptBin "dev-docker" ''
          export PATH="/usr/local/bin:$HOME/.orbstack/bin:$PATH"
          start_time="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
          compose_args=(-f docker-compose.yml -f docker-compose.dev-docker.yml)

          if ! command -v docker >/dev/null 2>&1; then
            echo "ERROR: docker コマンドが見つかりません。OrbStack または Docker Desktop が起動しているか確認してください。"
            exit 1
          fi

          if [ -z "$POSTGRES_PASSWORD" ]; then
            export POSTGRES_PASSWORD="dev"
          fi

          trap 'echo ""; echo "Stopping services..."; docker compose stop' EXIT INT TERM

          echo "Building judge image..."
          docker compose "''${compose_args[@]}" build judge

          echo "Starting PostgreSQL container..."
          docker compose "''${compose_args[@]}" up -d db

          echo "Waiting for database to be ready..."
          until docker compose "''${compose_args[@]}" exec -T db pg_isready -U mikan -d mikan_judge >/dev/null 2>&1; do
            sleep 1
          done

          echo "Starting judge container..."
          docker compose "''${compose_args[@]}" up -d judge

          echo "Waiting for judge to respond..."
          until curl --silent --fail "http://localhost:${"\${PORT:-3000}"}/" >/dev/null 2>&1; do
            sleep 1
          done

          judge_container_id="$(docker compose "''${compose_args[@]}" ps -q judge)"
          if [ -z "$judge_container_id" ]; then
            echo "ERROR: judge コンテナ ID を取得できませんでした。"
            exit 1
          fi

          echo "Listening on http://localhost:${"\${PORT:-3000}"}"
          echo "(Ctrl+C to stop)"
          docker logs --since "$start_time" -f "$judge_container_id"
        '';

        dev = pkgs.writeShellScriptBin "dev" ''
          # OrbStack / Docker Desktop がインストールした docker を優先する。
          # pkgs.docker（Nix製）はソケットパスを知らないため PATH の先頭に追加。
          export PATH="/usr/local/bin:$HOME/.orbstack/bin:$PATH"

          if ! command -v docker >/dev/null 2>&1; then
            echo "ERROR: docker コマンドが見つかりません。OrbStack または Docker Desktop が起動しているか確認してください。"
            exit 1
          fi

          # POSTGRES_PASSWORD が未設定の場合は開発用デフォルト値を使用する。
          # 設定ファイル（YAML 等）にハードコードせずシェル側で解決する。
          if [ -z "$POSTGRES_PASSWORD" ]; then
            export POSTGRES_PASSWORD="dev"
          fi

          echo "Starting database container..."
          docker compose up -d db

          echo "Waiting for database to be ready..."
          until docker compose exec -T db pg_isready -U mikan -d mikan_judge >/dev/null 2>&1; do
            sleep 1
          done

          db-migrate

          trap 'echo "Stopping database container..."; docker compose stop db' EXIT INT TERM
          cargo watch -x run
        '';

      in
      {
        # ── パッケージ出力 ───────────────────────────────────────
        packages = {
          default = rustPkg;
        } // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          # Linux 環境または CI でのみ使用可能。
          # $ nix build .#dockerImage && docker load < result
          dockerImage = pkgs.dockerTools.buildLayeredImage {
            name = "mikan-judge";
            tag  = "latest";

            contents = [
              appFiles
              pkgs.coreutils
              pkgs.bashInteractive
              pkgs.which
              pkgs.gcc                           # C++ 提出のコンパイル
              pkgs.jdk25_headless               # Java 提出のコンパイル・実行
              pkgs.go_1_26                      # Go 提出のコンパイル
              pkgs.python3                        # Python 提出の実行
              pkgs.pypy3                          # PyPy 提出の実行
              pkgs.rust-bin.stable."1.94.1".default # Rust 提出のコンパイル
              pkgs.libseccomp
              pkgs.cacert
            ];

            config = {
              Cmd       = [ "${rustPkg}/bin/mikan-judge" ];
              WorkingDir = "/app";
              ExposedPorts."3000/tcp" = {};
              Env = [
                "RUST_LOG=info"
                "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                "PATH=${pkgs.lib.makeBinPath [
                  rustPkg
                  pkgs.coreutils
                  pkgs.bashInteractive
                  pkgs.which
                  pkgs.gcc
                  pkgs.jdk25_headless
                  pkgs.go_1_26
                  pkgs.python3
                  pkgs.pypy3
                  pkgs.rust-bin.stable."1.94.1".default
                ]}"
              ];
            };
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain
            pkgs.gcc
            pkgs.gdb
            pkgs.pkg-config
            pkgs.openssl.dev
            pkgs.cargo-watch
            pkgs.sqlx-cli
            pkgs.jdk25_headless
            pkgs.go_1_26
            pkgs.python3
            pkgs.pypy3
            dbMigrate
            dev
            devDocker
          ] ++ linuxPkgs;

          PKG_CONFIG_PATH = pkgs.lib.makeSearchPath "lib/pkgconfig" (
            [ pkgs.openssl.dev ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.libseccomp.dev ]
          );

          shellHook = ''
            echo "mikan-judge dev shell"
            echo "  Rust : $(rustc --version)"
            echo "  g++  : $(g++ --version | head -1)"
            echo "  Java : $(${pkgs.jdk25_headless}/bin/javac --version)"
            echo "  Go   : $(${pkgs.go_1_26}/bin/go version)"
            echo "  CPy  : $(${pkgs.python3}/bin/python3 --version)"
            echo "  PyPy : $(${pkgs.pypy3}/bin/pypy3 --version 2>&1 | paste -sd ' ' -)"
            echo ""
            echo "Start: dev"
            echo "Linux Docker judge: dev-docker"

            # PostgreSQL は Docker コンテナで動かす（docker-compose.yml の db サービス）
            # POSTGRES_PASSWORD が未設定の場合は開発用デフォルト値をシェル側で設定する
            if [ -z "$POSTGRES_PASSWORD" ]; then
              export POSTGRES_PASSWORD="dev"
            fi
            export DATABASE_URL="postgresql://mikan:$POSTGRES_PASSWORD@localhost:5432/mikan_judge"
            if [ -z "$RUST_LOG" ]; then
              export RUST_LOG="info"
            fi

          '';
        };
      }
    );
}
