{ inputs, ... }:
{
  perSystem =
    {
      config,
      pkgs,
      lib,
      system,
      minneCtx,
      ...
    }:
    let
      inherit (minneCtx) rustToolchain surrealdbPkg devGraphicsLibs;

      devDefaults = import ./dev-defaults.nix;
      inherit (devDefaults) surreal minio app;

      ortDylib =
        if pkgs.stdenv.isDarwin then
          "${pkgs.onnxruntime}/lib/libonnxruntime.dylib"
        else
          "${pkgs.onnxruntime}/lib/libonnxruntime.so";

      processComposeTemplate = builtins.readFile ../dev/process-compose.yaml;

      minioAddress = lib.removePrefix "http://" minio.endpoint;

      processComposeFile = pkgs.writeText "minne-process-compose.yaml" (
        lib.replaceStrings
          [
            "@SURREALDB@"
            "@TAILWIND@"
            "@MINIO@"
            "@MC@"
            "@SURREAL_BIND@"
            "@SURREAL_USER@"
            "@SURREAL_PASS@"
            "@MINIO_ADDRESS@"
            "@MINIO_ENDPOINT@"
            "@MINIO_USER@"
            "@MINIO_PASSWORD@"
            "@MINIO_REGION@"
            "@MINIO_BUCKET@"
          ]
          [
            "${surrealdbPkg}/bin/surreal"
            "${pkgs.tailwindcss_4}/bin/tailwindcss"
            "${pkgs.minio}/bin/minio"
            "${pkgs.minio-client}/bin/mc"
            "${surreal.host}:${toString surreal.port}"
            surreal.user
            surreal.pass
            minioAddress
            minio.endpoint
            minio.accessKey
            minio.secretKey
            minio.region
            minio.bucket
          ]
          processComposeTemplate
      );

      # Shared project-local socket so `up` and `down` always target the same
      # process-compose instance regardless of the caller's working directory.
      processComposeSocket = ".data/process-compose.sock";

      # Resolve the writable git checkout root; never the read-only flake store.
      resolveRoot = name: ''
        root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
        if [ ! -w "$root" ]; then
          echo "${name}: workspace is not writable: $root" >&2
          exit 1
        fi
        cd "$root"
      '';

      processComposeRunner = pkgs.writeShellScriptBin "minne-dev-up" ''
        set -euo pipefail
        ${resolveRoot "minne-dev-up"}
        mkdir -p .data/minio database html-router/assets
        exec ${pkgs.process-compose}/bin/process-compose up \
          -u ${processComposeSocket} -U \
          -f ${processComposeFile} "$@"
      '';

      processComposeDownRunner = pkgs.writeShellScriptBin "minne-dev-down" ''
        set -euo pipefail
        ${resolveRoot "minne-dev-down"}
        exec ${pkgs.process-compose}/bin/process-compose down \
          -u ${processComposeSocket} -U "$@"
      '';

      moldFlags = if pkgs.stdenv.isLinux then "-C link-arg=-fuse-ld=mold" else "";

      cargoDenyRunner = pkgs.writeShellScriptBin "cargo-deny-check" ''
        export PATH="${
          lib.makeBinPath [
            rustToolchain.toolchain
            pkgs.cargo-deny
          ]
        }:$PATH"
        exec cargo deny check licenses bans sources
      '';

      preCommitCheck = inputs.git-hooks.lib.${system}.run {
        src = inputs.self;
        hooks = {
          rustfmt.enable = true;
          clippy = {
            enable = true;
            settings.allFeatures = true;
          };
          nixfmt.enable = true;
          deadnix.enable = true;
          statix.enable = true;
          actionlint.enable = true;
          typos.enable = true;
          cargo-deny = {
            enable = true;
            name = "cargo-deny";
            description = "Check dependency licenses, bans, and sources";
            entry = "${cargoDenyRunner}/bin/cargo-deny-check";
            files = "(Cargo\\.(toml|lock)|deny\\.toml)$";
            pass_filenames = false;
          };
        };
        tools = {
          cargo = rustToolchain.toolchain;
          clippy = rustToolchain.toolchain;
          rustfmt = rustToolchain.toolchain;
          inherit (pkgs) nixfmt;
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
        config.treefmt.build.wrapper
        rustToolchain.toolchain
        surrealdbPkg
        processComposeRunner
        processComposeDownRunner
        pkgs.process-compose
        pkgs.minio
        pkgs.minio-client
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
        pkgs.deadnix
        pkgs.statix
        pkgs.actionlint
        pkgs.typos
        pkgs.cargo-deny
        cargoDenyRunner
      ];

      loadLocalEnv = ''
        root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
        local_env="$root/.env.local"
        if [ -f "$local_env" ]; then
          set -a
          # shellcheck source=/dev/null
          source "$local_env"
          set +a
        fi
      '';

      devEnv = {
        NIX_CFLAGS_COMPILE = "-Wno-error=cpp";
        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        LD_LIBRARY_PATH = lib.makeLibraryPath devGraphicsLibs;
        ORT_DYLIB_PATH = ortDylib;
        RUSTFLAGS = moldFlags;

        # Minne app — matches `nix run .#dev` services; override via `.env.local`.
        OPENAI_API_KEY = app.openaiApiKey;
        SURREALDB_ADDRESS = "ws://${surreal.host}:${toString surreal.port}";
        SURREALDB_USERNAME = surreal.user;
        SURREALDB_PASSWORD = surreal.pass;
        SURREALDB_NAMESPACE = surreal.namespace;
        SURREALDB_DATABASE = surreal.database;
        HTTP_PORT = toString app.httpPort;
        DATA_DIR = app.dataDir;
        EMBEDDING_BACKEND = app.embeddingBackend;
        PDF_INGEST_MODE = app.pdfIngestMode;
        STORAGE = app.storage;
        RUST_LOG = app.rustLog;

        S3_ENDPOINT = minio.endpoint;
        S3_BUCKET = minio.bucket;
        S3_REGION = minio.region;
        AWS_ACCESS_KEY_ID = minio.accessKey;
        AWS_SECRET_ACCESS_KEY = minio.secretKey;
        MINNE_TEST_S3_ENDPOINT = minio.endpoint;
        MINNE_TEST_S3_BUCKET = minio.bucket;
      };
    in
    {
      apps.dev = {
        type = "app";
        program = "${processComposeRunner}/bin/minne-dev-up";
        meta.description = "Start local dev services (SurrealDB, MinIO, Tailwind)";
      };

      apps.dev-down = {
        type = "app";
        program = "${processComposeDownRunner}/bin/minne-dev-down";
        meta.description = "Stop local dev services started by minne-dev-up";
      };

      devShells.default = pkgs.mkShell {
        packages = devPackages;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        env = devEnv;
        shellHook = ''
          ${preCommitCheck.shellHook}
          ${installGitHooks}
          ${loadLocalEnv}
          echo "Minne dev shell (fenix ${rustToolchain.rustVersion})"
          echo "  nix run .#dev          # or: minne-dev-up"
          echo "  nix run .#dev-down     # or: minne-dev-down"
          echo "  nix fmt                # rustfmt + nixfmt"
          echo "  cargo run -p main      # env from dev shell + optional .env.local"
          echo "  overrides: cp .env.local.example .env.local"
          echo "  cargo test --workspace"
          echo "  nix flake check"
        '';
      };
    };
}
