{
  description = "Minne application flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    crane,
    fenix,
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
      minneVersion = "1.0.5";
      mozjsRelease = "mozjs-sys-v140.10.1-0";

      mozjsTarget =
        {
          "x86_64-linux" = "x86_64-unknown-linux-gnu";
          "aarch64-linux" = "aarch64-unknown-linux-gnu";
          "aarch64-darwin" = "aarch64-apple-darwin";
          "x86_64-darwin" = "x86_64-apple-darwin";
        }
        .${system}
        or (throw "mozjs prebuilt archive not configured for system ${system}");

      mozjsHashes = {
        "x86_64-unknown-linux-gnu" = "sha256-e5kW8HTg6Hrd3sGgU9bqFNTTf7wJCChFOwKE3xyYT4Q=";
        "aarch64-unknown-linux-gnu" = "sha256-VXrcktvjSH+14tO9Kzx+n9f/9ZQGAzfEsniiT+xKT6Q=";
        "aarch64-apple-darwin" = "sha256-T3y73nVic6R60keUpmVRFe110Eh7AcE/VwZQWXRU9A0=";
        "x86_64-apple-darwin" = "sha256-4v6f6c1OwYdg1FKnFfdLEsrRdyghcxup4gF7ioTZzm4=";
      };

      # Pre-download mozjs binary archive for mozjs_sys (servo dep).
      # When updating mozjs_sys version in Cargo.lock, update mozjsRelease + hashes.
      mozjsArchive = pkgs.fetchurl {
        url = "https://github.com/servo/mozjs/releases/download/${mozjsRelease}/libmozjs-${mozjsTarget}.tar.gz";
        hash = mozjsHashes.${mozjsTarget} or (throw "missing mozjs hash for ${mozjsTarget}");
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

      # Build *just* the cargo dependencies using a dummy source so that source
      # code changes don't invalidate the cached dependency layer.
      cargoArtifacts = craneLib.buildDepsOnly (commonArgs
        // {
          pname = "minne";
          cargoExtraArgs = "--workspace";
          doCheck = false;
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

              postInstall =
                lib.optionalString pkgs.stdenv.isLinux ''
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
                ''
                + lib.optionalString pkgs.stdenv.isDarwin ''
                  for b in main worker server; do
                    if [ -x "$out/bin/$b" ]; then
                      wrapProgram $out/bin/$b \
                        --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
                    fi
                  done
                '';
            })
        else throw "pkgs.onnxruntime.version (${pkgs.onnxruntime.version}) must match ortVersion in flake.nix (${ortVersion})";

      targetTriple = pkgs.stdenv.hostPlatform.config;

      releaseCommonArgs = {
        inherit minneVersion targetTriple;
        bzip2 = pkgs.bzip2.out;
        brotli = pkgs.brotli.lib;
        srcRoot = ./.;
      };

      minne-release =
        if pkgs.stdenv.isLinux
        then
          pkgs.callPackage ./nix/minne-release.nix (releaseCommonArgs // {
            platform = "linux";
            inherit minne-pkg;
          })
        else if pkgs.stdenv.isDarwin
        then
          pkgs.callPackage ./nix/minne-release.nix (releaseCommonArgs // {
            platform = "darwin";
            inherit minne-pkg;
          })
        else null;

      windowsTarget = "x86_64-pc-windows-msvc";

      windowsRustToolchain =
        if system == "x86_64-linux"
        then
          let
            fenixPkgs = fenix.packages.${system};
          in
          fenixPkgs.combine [
            fenixPkgs.stable.defaultToolchain
            fenixPkgs.targets.${windowsTarget}.stable.rust-std
          ]
        else null;

      windowsCraneLib =
        if system == "x86_64-linux"
        then craneLib.overrideToolchain (_: windowsRustToolchain)
        else craneLib;

      mozjsArchiveWindows = pkgs.fetchurl {
        url = "https://github.com/servo/mozjs/releases/download/${mozjsRelease}/libmozjs-${windowsTarget}.tar.gz";
        hash = "sha256-nEX55a4vZJGxlDMCea9TEee60HiNe/yQzXtUqMlaM3c=";
      };

      ortArchiveWindows = pkgs.fetchurl {
        url = "https://github.com/microsoft/onnxruntime/releases/download/v${ortVersion}/onnxruntime-win-x64-${ortVersion}.zip";
        hash = "sha256-CzjfmvIYNOQec9YC2Q21ywbb0cphiUi48dZtYHrJ880=";
      };

      windowsCross = pkgs.callPackage ./nix/windows-cross.nix {};

      inherit (windowsCross) clangClWrapper xwinCargoCache;

      # blake3's build.rs only enables MSVC asm when CC_x86_64_pc_windows_msvc is exactly
      # "cl" or "cl.exe" (not a store path). Route through the clang-cl wrapper.
      # cc-rs invokes ml64.exe for MSVC asm; llvm-ml64 is ml64-compatible.
      msvcShim = pkgs.symlinkJoin {
        name = "minne-msvc-shim";
        paths = [
          (pkgs.writeShellScriptBin "cl.exe" ''
            exec ${clangClWrapper} "$@"
          '')
          (pkgs.writeShellScriptBin "ml64.exe" ''
            exec ${pkgs.llvmPackages.llvm}/bin/llvm-ml64 "$@"
          '')
        ];
      };

      # Offline MSVC env (nixpkgs cargo-xwin lacks `cache` and tries to download CRT in sandbox).
      xwinSetup = pkgs.writeShellScript "minne-xwin-setup" ''
        set -eo pipefail

        cache=${xwinCargoCache}
        crt="$cache/xwin/crt"
        sdk="$cache/xwin/sdk"

        export PATH="${msvcShim}/bin:${pkgs.llvmPackages.clang-unwrapped}/bin:${pkgs.llvmPackages.lld}/bin:${pkgs.llvmPackages.llvm}/bin:$PATH"

        # Host build scripts (webrender, etc.) run on Linux and need libstdc++ at runtime.
        export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

        export AR_x86_64_pc_windows_msvc=${pkgs.llvmPackages.llvm}/bin/llvm-lib
        export BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc="-I$crt/include -I$sdk/include/ucrt -I$sdk/include/um -I$sdk/include/shared -I$sdk/include/winrt"
        export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=${pkgs.llvmPackages.lld}/bin/lld-link
        export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C linker-flavor=lld-link -Lnative=$crt/lib/x86_64 -Lnative=$sdk/lib/um/x86_64 -Lnative=$sdk/lib/ucrt/x86_64"
        export CC_x86_64_pc_windows_msvc=cl.exe
        export CXX_x86_64_pc_windows_msvc=cl.exe
        export REAL_CLANG_CL=${pkgs.llvmPackages.clang-unwrapped}/bin/clang-cl
        export REAL_LLD_LINK=${pkgs.llvmPackages.lld}/bin/lld-link

        _imsvc="--target=x86_64-pc-windows-msvc -Wno-unused-command-line-argument -fuse-ld=lld-link /imsvc $crt/include /imsvc $sdk/include/ucrt /imsvc $sdk/include/um /imsvc $sdk/include/shared /imsvc $sdk/include/winrt"
        export CFLAGS_x86_64_pc_windows_msvc="$_imsvc"
        export CXXFLAGS_x86_64_pc_windows_msvc="$_imsvc /EHsc"
        export CL_FLAGS="--target=x86_64-pc-windows-msvc -Wno-unused-command-line-argument -fuse-ld=lld-link /imsvc $crt/include /imsvc $sdk/include/ucrt /imsvc $sdk/include/um /imsvc $sdk/include/shared /imsvc $sdk/include/winrt"

        export CMAKE_GENERATOR=Ninja
        export CMAKE_SYSTEM_NAME=Windows
        export CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc="$cache/cmake/clang-cl/x86_64-pc-windows-msvc-toolchain.cmake"

        export LIB="$crt/lib/x86_64;$sdk/lib/um/x86_64;$sdk/lib/ucrt/x86_64"
        export RCFLAGS="-I$crt/include -I$sdk/include/ucrt -I$sdk/include/um -I$sdk/include/shared -I$sdk/include/winrt"
        export TARGET_AR=${pkgs.llvmPackages.llvm}/bin/llvm-lib
        export TARGET_CC=${pkgs.llvmPackages.clang-unwrapped}/bin/clang-cl
        export TARGET_CXX=${pkgs.llvmPackages.clang-unwrapped}/bin/clang-cl
        export WINEDEBUG=-all
      '';

      windowsCommonArgs =
        commonArgs
        // {
          MOZJS_ARCHIVE = "${mozjsArchiveWindows}";
          CARGO_BUILD_TARGET = windowsTarget;
          doIncludeCrossToolchainEnv = false;
          env.CARGO_PROFILE = "dist";
          buildInputs = [
            pkgs.openssl
            pkgs.fontconfig
            pkgs.libclang.lib
          ];
          nativeBuildInputs =
            commonArgs.nativeBuildInputs
            ++ [
              pkgs.llvmPackages.llvm
              pkgs.llvmPackages.clang-unwrapped
              pkgs.llvmPackages.lld
              pkgs.stdenv.cc.cc.lib # host build scripts (e.g. webrender) link libstdc++
            ];
        };

      windowsCargoArtifacts =
        if system == "x86_64-linux"
        then
          windowsCraneLib.buildDepsOnly (windowsCommonArgs
            // {
              pname = "minne";
              cargoExtraArgs = "--workspace";
              doCheck = false;
              preBuild = "source ${xwinSetup}";
            })
        else null;

      minne-pkg-windows =
        if system == "x86_64-linux"
        then
          windowsCraneLib.buildPackage (windowsCommonArgs
            // {
              pname = "minne-windows";
              version = minneVersion;
              cargoArtifacts = windowsCargoArtifacts;
              cargoExtraArgs = "--target ${windowsTarget} -p main --bin main --bin server --bin worker";
              doCheck = false;
              doInstallCargoArtifacts = false;
              preBuild = "source ${xwinSetup}";
              installPhaseCommand = ''
                mkdir -p "$out/bin"
                for b in main server worker; do
                  install -m 755 "target/${windowsTarget}/dist/$b.exe" "$out/bin/$b.exe"
                done
              '';
            })
        else null;

      minne-release-windows =
        if system == "x86_64-linux"
        then
          pkgs.callPackage ./nix/minne-release.nix (releaseCommonArgs // {
            platform = "windows";
            inherit minne-pkg-windows ortArchiveWindows;
            targetTriple = windowsTarget;
          })
        else null;

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
      }
      // lib.optionalAttrs (minne-release != null) {
        minne-release = minne-release;
      }
      // lib.optionalAttrs (minne-release-windows != null) {
        inherit xwinCargoCache;
        minne-release-windows = minne-release-windows;
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
            buildInputs = commonArgs.buildInputs ++ [pkgs.cacert];
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
