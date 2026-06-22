{
  description = "Minne application flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    crane,
  }: let
    inherit (nixpkgs.legacyPackages.x86_64-linux) lib;
    ortVersion = "1.23.2";
  in
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      lib = pkgs.lib;
      craneLib = crane.mkLib pkgs;
      libExt =
        if pkgs.stdenv.isDarwin
        then "dylib"
        else "so";
      minneVersion = "1.0.3";

      # Pre-download mozjs binary archive for mozjs_sys (servo dep).
      # When updating mozjs_sys version in Cargo.lock, update this URL too.
      mozjsArchive = pkgs.fetchurl {
        url = "https://github.com/servo/mozjs/releases/download/mozjs-sys-v140.10.1-0/libmozjs-x86_64-unknown-linux-gnu.tar.gz";
        hash = "sha256-e5kW8HTg6Hrd3sGgU9bqFNTTf7wJCChFOwKE3xyYT4Q=";
      };

      # Extra paths (common/db, html-router/templates, html-router/assets) are
      # embedded at compile time via include_dir! / minijinja_embed.
      commonArgs = {
        version = minneVersion;
        src = lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            craneLib.filterCargoSources path type
            || lib.any (x: lib.hasPrefix (toString x) (toString path)) [
              (toString ./Cargo.lock)
              (toString ./common/db)
              (toString ./html-router/templates)
              (toString ./html-router/assets)
            ];
        };
        strictDeps = true;

        buildInputs = [
          pkgs.openssl
          pkgs.libglvnd
          pkgs.onnxruntime
          pkgs.fontconfig # .pc for yeslogic-fontconfig-sys (servo dep)
          pkgs.libclang.lib # libclang.so for bindgen (servo dep)
        ];

        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.rustfmt
          pkgs.makeWrapper
          pkgs.python3 # needed by servo's stylo crate build.rs
          pkgs.llvmPackages.llvm # llvm-objdump for mozjs_sys (servo dep)
          pkgs.rustPlatform.bindgenHook # configures bindgen (servo deps)
        ];

        # Provide pre-downloaded mozjs archive so it doesn't need network
        MOZJS_ARCHIVE = "${mozjsArchive}";
      };

      # cargoBuild (not buildDepsOnly) avoids mkDummySrc breaking native build scripts.
      cargoArtifacts = craneLib.cargoBuild (commonArgs
        // {
          cargoArtifacts = null;
          pname = "minne-deps";
          cargoExtraArgs = "--workspace";
          doCheck = false;
          doInstallCargoArtifacts = true;
          installPhaseCommand = "";
        });

      minne-pkg =
        if pkgs.onnxruntime.version == ortVersion
        then
          craneLib.buildPackage (commonArgs
            // {
              pname = "minne";
              version = minneVersion;
              inherit cargoArtifacts;
              doCheck = false; # checks are in separate derivations
              doInstallCargoArtifacts = true; # for reuse by check derivations

              postInstall = ''
                wrapProgram $out/bin/main \
                  --prefix LD_LIBRARY_PATH : ${pkgs.libglvnd}/lib \
                  --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
                for b in worker server; do
                  if [ -x "$out/bin/$b" ]; then
                    wrapProgram $out/bin/$b \
                      --prefix LD_LIBRARY_PATH : ${pkgs.libglvnd}/lib \
                      --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
                  fi
                done
              '';
            })
        else throw "pkgs.onnxruntime.version (${pkgs.onnxruntime.version}) must match ortVersion in flake.nix (${ortVersion})";

      dockerImage = pkgs.dockerTools.buildLayeredImage {
        name = "minne";
        tag = minneVersion;
        created = "now";

        contents = [
          minne-pkg
          pkgs.cacert
          pkgs.bashInteractive
          pkgs.libglvnd
          pkgs.fontconfig.lib
          pkgs.freetype
          pkgs.stdenv.cc.cc.lib # libgomp (OpenMP) for ONNX Runtime
        ];

        maxLayers = 25;

        config = {
          Cmd = ["${minne-pkg}/bin/main"];
          Env = [
            "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-certificates.crt"
            "ORT_DYLIB_PATH=${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}"
          ];
          ExposedPorts = {"3000/tcp" = {};};
          User = "appuser";
        };
      };
    in {
      packages = {
        inherit minne-pkg dockerImage;
        default = minne-pkg;
      };

      apps = {
        main = {
          type = "app";
          program = "${minne-pkg}/bin/main";
          meta.description = "Minne main server — API, web UI, and background worker";
        };
        worker = {
          type = "app";
          program = "${minne-pkg}/bin/worker";
          meta.description = "Minne standalone background worker (ingestion, indexing, maintenance)";
        };
        server = {
          type = "app";
          program = "${minne-pkg}/bin/server";
          meta.description = "Minne API-only server (no background worker)";
        };
        default = {
          type = "app";
          program = "${minne-pkg}/bin/main";
          meta.description = "Minne main server — API, web UI, and background worker";
        };
      };

      checks = {
        ortVersion = pkgs.runCommand "ort-version-check" {} ''
          if [ "${pkgs.onnxruntime.version}" != "${ortVersion}" ]; then
            echo "pkgs.onnxruntime.version is ${pkgs.onnxruntime.version}, but flake pins ${ortVersion}" >&2
            echo "Update ortVersion in flake.nix or wait for nixpkgs to catch up." >&2
            exit 1
          fi
          touch $out
        '';

        minne-clippy = craneLib.cargoClippy (commonArgs
          // {
            cargoArtifacts = minne-pkg;
            pname = "minne";
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

        minne-test = craneLib.cargoTest (commonArgs
          // {
            cargoArtifacts = minne-pkg;
            pname = "minne";
            buildInputs = commonArgs.buildInputs ++ [ pkgs.cacert ];
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-certificates.crt";
            cargoTestExtraArgs = "--lib --bins";
          });

        minne-fmt = craneLib.cargoFmt {
          pname = "minne-fmt";
          version = minneVersion;
          src = craneLib.cleanCargoSource ./.;
        };
      };
    })
    // {
      lib = {
        inherit ortVersion;
      };
    };
}
