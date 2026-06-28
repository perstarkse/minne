# Shared build context for packages, checks, and the dev shell.
{
  inputs,
  pkgs,
  system,
  src,
  ortVersion,
  toolchainFile,
}:
let
  lib = pkgs.lib;
  inherit (inputs) crane fenix;

  surrealdbPkg =
    if system == "x86_64-linux" then pkgs.callPackage ./surrealdb-binary.nix { } else pkgs.surrealdb;

  rustToolchain = import ./rust-toolchain.nix {
    inherit fenix system toolchainFile;
  };

  craneLib = rustToolchain.mkCraneLib pkgs (crane.mkLib pkgs);

  libExt = if pkgs.stdenv.isDarwin then "dylib" else "so";

  linuxRuntimeLibs = with pkgs; [
    libglvnd
    stdenv.cc.cc.lib
    zlib
    fontconfig.lib
    freetype
    openssl.out
    onnxruntime
  ];

  devGraphicsLibs = with pkgs; [
    wayland
    libxkbcommon
    pipewire
    libglvnd
  ];

  wrapLinuxBinary = libExt: ''
    wrapProgram $out/bin/main \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath linuxRuntimeLibs} \
      --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
    for b in worker server; do
      if [ -x "$out/bin/$b" ]; then
        wrapProgram $out/bin/$b \
          --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath linuxRuntimeLibs} \
          --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
      fi
    done
  '';

  minneVersion = "1.0.5";
  mozjsRelease = "mozjs-sys-v140.10.1-0";

  mozjsTarget =
    {
      "x86_64-linux" = "x86_64-unknown-linux-gnu";
      "aarch64-linux" = "aarch64-unknown-linux-gnu";
      "aarch64-darwin" = "aarch64-apple-darwin";
      "x86_64-darwin" = "x86_64-apple-darwin";
    }
    .${system} or (throw "mozjs prebuilt archive not configured for system ${system}");

  mozjsHashes = {
    "x86_64-unknown-linux-gnu" = "sha256-e5kW8HTg6Hrd3sGgU9bqFNTTf7wJCChFOwKE3xyYT4Q=";
    "aarch64-unknown-linux-gnu" = "sha256-VXrcktvjSH+14tO9Kzx+n9f/9ZQGAzfEsniiT+xKT6Q=";
    "aarch64-apple-darwin" = "sha256-T3y73nVic6R60keUpmVRFe110Eh7AcE/VwZQWXRU9A0=";
    "x86_64-apple-darwin" = "sha256-4v6f6c1OwYdg1FKnFfdLEsrRdyghcxup4gF7ioTZzm4=";
  };

  mozjsArchive = pkgs.fetchurl {
    url = "https://github.com/servo/mozjs/releases/download/${mozjsRelease}/libmozjs-${mozjsTarget}.tar.gz";
    hash = mozjsHashes.${mozjsTarget} or (throw "missing mozjs hash for ${mozjsTarget}");
  };

  commonArgs = {
    version = minneVersion;
    src = lib.cleanSourceWith {
      inherit src;
      filter =
        path: type:
        craneLib.filterCargoSources path type
        || lib.any (x: lib.hasPrefix (toString x) (toString path)) [
          (toString src + "/Cargo.lock")
          (toString src + "/common/db")
          (toString src + "/html-router/templates")
          (toString src + "/html-router/assets")
        ];
    };
    strictDeps = true;

    buildInputs = [
      pkgs.openssl
      pkgs.libglvnd
      pkgs.onnxruntime
      pkgs.fontconfig
      pkgs.libclang.lib
    ];

    nativeBuildInputs = [
      pkgs.pkg-config
      pkgs.rustfmt
      pkgs.makeWrapper
      pkgs.python3
      pkgs.llvmPackages.llvm
      pkgs.rustPlatform.bindgenHook
      pkgs.stdenv.cc.cc.lib
    ];

    MOZJS_ARCHIVE = "${mozjsArchive}";
    env.LD_LIBRARY_PATH = lib.makeLibraryPath linuxRuntimeLibs;
  };

  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      pname = "minne";
      cargoExtraArgs = "--workspace";
      doCheck = false;
    }
  );

  minne-pkg =
    if pkgs.onnxruntime.version == ortVersion then
      craneLib.buildPackage (
        commonArgs
        // {
          pname = "minne";
          version = minneVersion;
          inherit cargoArtifacts;
          doCheck = false;
          doInstallCargoArtifacts = true;

          postInstall =
            lib.optionalString pkgs.stdenv.isLinux (wrapLinuxBinary libExt)
            + lib.optionalString pkgs.stdenv.isDarwin ''
              for b in main worker server; do
                if [ -x "$out/bin/$b" ]; then
                  wrapProgram $out/bin/$b \
                    --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
                fi
              done
            '';
        }
      )
    else
      throw "pkgs.onnxruntime.version (${pkgs.onnxruntime.version}) must match ortVersion (${ortVersion})";

  targetTriple = pkgs.stdenv.hostPlatform.config;

  releaseCommonArgs = {
    inherit minneVersion targetTriple;
    bzip2 = pkgs.bzip2.out;
    brotli = pkgs.brotli.lib;
    srcRoot = src;
  };

  minne-release =
    if pkgs.stdenv.isLinux then
      pkgs.callPackage ./minne-release.nix (
        releaseCommonArgs
        // {
          platform = "linux";
          inherit minne-pkg;
        }
      )
    else if pkgs.stdenv.isDarwin then
      pkgs.callPackage ./minne-release.nix (
        releaseCommonArgs
        // {
          platform = "darwin";
          inherit minne-pkg;
        }
      )
    else
      null;

  windowsTarget = rustToolchain.windowsTarget;

  mozjsArchiveWindows = pkgs.fetchurl {
    url = "https://github.com/servo/mozjs/releases/download/${mozjsRelease}/libmozjs-${windowsTarget}.tar.gz";
    hash = "sha256-nEX55a4vZJGxlDMCea9TEee60HiNe/yQzXtUqMlaM3c=";
  };

  ortArchiveWindows = pkgs.fetchurl {
    url = "https://github.com/microsoft/onnxruntime/releases/download/v${ortVersion}/onnxruntime-win-x64-${ortVersion}.zip";
    hash = "sha256-CzjfmvIYNOQec9YC2Q21ywbb0cphiUi48dZtYHrJ880=";
  };

  windowsCross = pkgs.callPackage ./windows-cross.nix { };

  inherit (windowsCross) clangClWrapper xwinCargoCache;

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

  xwinSetup = pkgs.writeShellScript "minne-xwin-setup" ''
    set -eo pipefail

    cache=${xwinCargoCache}
    crt="$cache/xwin/crt"
    sdk="$cache/xwin/sdk"

    export PATH="${msvcShim}/bin:${pkgs.llvmPackages.clang-unwrapped}/bin:${pkgs.llvmPackages.lld}/bin:${pkgs.llvmPackages.llvm}/bin:$PATH"
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

  windowsCommonArgs = commonArgs // {
    MOZJS_ARCHIVE = "${mozjsArchiveWindows}";
    CARGO_BUILD_TARGET = windowsTarget;
    doIncludeCrossToolchainEnv = false;
    env.CARGO_PROFILE = "dist";
    buildInputs = [
      pkgs.openssl
      pkgs.fontconfig
      pkgs.libclang.lib
    ];
    nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
      pkgs.llvmPackages.llvm
      pkgs.llvmPackages.clang-unwrapped
      pkgs.llvmPackages.lld
      pkgs.stdenv.cc.cc.lib
    ];
  };

  windowsCargoArtifacts =
    if system == "x86_64-linux" then
      craneLib.buildDepsOnly (
        windowsCommonArgs
        // {
          pname = "minne";
          cargoExtraArgs = "--workspace";
          doCheck = false;
          preBuild = "source ${xwinSetup}";
        }
      )
    else
      null;

  minne-pkg-windows =
    if system == "x86_64-linux" then
      craneLib.buildPackage (
        windowsCommonArgs
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
        }
      )
    else
      null;

  minne-release-windows =
    if system == "x86_64-linux" then
      pkgs.callPackage ./minne-release.nix (
        releaseCommonArgs
        // {
          platform = "windows";
          inherit minne-pkg-windows ortArchiveWindows;
          targetTriple = windowsTarget;
        }
      )
    else
      null;

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
      pkgs.stdenv.cc.cc.lib
    ];

    maxLayers = 25;

    config = {
      Cmd = [ "${minne-pkg}/bin/main" ];
      Env = [
        "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-certificates.crt"
        "ORT_DYLIB_PATH=${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}"
      ];
      ExposedPorts = {
        "3000/tcp" = { };
      };
      User = "appuser";
    };
  };

  vmSmokeTest =
    if system == "x86_64-linux" then
      pkgs.callPackage ./vm-smoke-test.nix {
        inherit minne-pkg;
        surrealdb = surrealdbPkg;
      }
    else
      null;
in
{
  inherit
    src
    ortVersion
    lib
    libExt
    craneLib
    rustToolchain
    surrealdbPkg
    linuxRuntimeLibs
    devGraphicsLibs
    commonArgs
    minneVersion
    minne-pkg
    minne-pkg-windows
    minne-release
    minne-release-windows
    dockerImage
    vmSmokeTest
    xwinCargoCache
    ;
}
