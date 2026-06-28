# Local development shell, git hooks, and process-compose runner.
{ inputs, ... }:
{
  perSystem =
    {
      pkgs,
      lib,
      system,
      minneCtx,
      ...
    }:
    let
      inherit (minneCtx) rustToolchain surrealdbPkg devGraphicsLibs;

      ortDylib =
        if pkgs.stdenv.isDarwin then
          "${pkgs.onnxruntime}/lib/libonnxruntime.dylib"
        else
          "${pkgs.onnxruntime}/lib/libonnxruntime.so";

      processComposeFile = pkgs.writeText "minne-process-compose.yaml" ''
        version: "0.5"

        environment:
          - MINIO_ROOT_USER=minioadmin
          - MINIO_ROOT_PASSWORD=minioadmin
          - MINIO_REGION=us-east-1

        processes:
          surreal_db:
            command: |
              mkdir -p database
              exec ${surrealdbPkg}/bin/surreal start \
                --bind 127.0.0.1:8000 \
                --log info \
                --user root_user \
                --pass root_password \
                rocksdb:database/database.db
            availability:
              restart: on_failure

          tailwind:
            command: ${pkgs.tailwindcss_4}/bin/tailwindcss --cwd html-router -i app.css -o assets/style.css --watch=always
            availability:
              restart: on_failure

          minio:
            command: |
              mkdir -p .data/minio
              exec ${pkgs.minio}/bin/minio server .data/minio \
                --address 127.0.0.1:19000 \
                --console-address 127.0.0.1:19001
            availability:
              restart: on_failure

          minio_setup:
            command: |
              for _ in $(seq 1 30); do
                if ${pkgs.minio-client}/bin/mc alias set local http://127.0.0.1:19000 minioadmin minioadmin 2>/dev/null; then
                  ${pkgs.minio-client}/bin/mc mb local/minne-tests --ignore-existing
                  exit 0
                fi
                sleep 1
              done
              echo "minio did not become ready" >&2
              exit 1
            depends_on:
              minio:
                condition: process_started
      '';

      processComposeRunner = pkgs.writeShellScriptBin "minne-dev-up" ''
        set -euo pipefail
        root="''${MINNE_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
        cd "$root"
        exec ${pkgs.process-compose}/bin/process-compose up -f ${processComposeFile} "$@"
      '';

      moldFlags = if pkgs.stdenv.isLinux then "-C link-arg=-fuse-ld=mold" else "";

      preCommitCheck = inputs.git-hooks.lib.${system}.run {
        src = inputs.self;
        hooks = {
          rustfmt.enable = true;
          clippy = {
            enable = true;
            settings.allFeatures = true;
          };
          nixfmt.enable = true;
        };
        tools = {
          cargo = rustToolchain.toolchain;
          clippy = rustToolchain.toolchain;
          rustfmt = rustToolchain.toolchain;
          nixfmt = pkgs.nixfmt;
        };
      };

      installGitHooks = ''
        legacy="$(git rev-parse --git-path hooks/pre-commit.legacy 2>/dev/null || true)"
        if [ -n "$legacy" ] && [ -f "$legacy" ]; then
          rm -f "$legacy"
        fi
        # prek migration can leave core.hooksPath set; pre-commit refuses to install then.
        hooks_path="$(git config --local --get core.hooksPath 2>/dev/null || true)"
        if [ -n "$hooks_path" ]; then
          git config --local --unset-all core.hooksPath
        fi
      '';

      devPackages = [
        rustToolchain.toolchain
        surrealdbPkg
        processComposeRunner
        pkgs.process-compose
        pkgs.minio
        pkgs.minio-client
        pkgs.openssl
        pkgs.nodejs
        pkgs.watchman
        pkgs.vscode-langservers-extracted
        pkgs.cargo-xwin
        pkgs.clang
        pkgs.onnxruntime
        pkgs.cargo-watch
        pkgs.tailwindcss_4
        pkgs.python3
        pkgs.fontconfig
        pkgs.fontconfig.dev
        pkgs.libGL
        pkgs.libGLU
        pkgs.libclang
        pkgs.mold
        pkgs.nixfmt
      ];

      devEnv = {
        NIX_CFLAGS_COMPILE = "-Wno-error=cpp";
        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        LD_LIBRARY_PATH = lib.makeLibraryPath devGraphicsLibs;
        ORT_DYLIB_PATH = ortDylib;
        S3_ENDPOINT = "http://127.0.0.1:19000";
        S3_BUCKET = "minne-tests";
        MINNE_TEST_S3_ENDPOINT = "http://127.0.0.1:19000";
        MINNE_TEST_S3_BUCKET = "minne-tests";
        RUSTFLAGS = moldFlags;
        MINNE_ROOT = "${inputs.self}";
      };
    in
    {
      packages.process-compose-runner = processComposeRunner;

      apps.dev = {
        type = "app";
        program = "${processComposeRunner}/bin/minne-dev-up";
        meta.description = "Start local dev services (SurrealDB, MinIO, Tailwind)";
      };

      devShells.default = pkgs.mkShell {
        packages = devPackages;
        env = devEnv;
        shellHook = ''
          ${preCommitCheck.shellHook}
          ${installGitHooks}
          echo "Minne dev shell (fenix ${rustToolchain.rustVersion})"
          echo "  nix run .#dev          # or: minne-dev-up"
          echo "  cargo test --workspace"
          echo "  nix flake check"
        '';
      };
    };
}
