{
  pkgs,
  system,
  ortVersion,
  versions,
  craneLib,
  rustToolchain,
  commonArgs,
  minneVersion,
  releaseCommonArgs,
}:
let
  inherit (versions) mozjsRelease mozjsArchiveWindowsHash ortArchiveWindowsHash;
  inherit (rustToolchain) windowsTarget;

  mozjsArchiveWindows = pkgs.fetchurl {
    url = "https://github.com/servo/mozjs/releases/download/${mozjsRelease}/libmozjs-${windowsTarget}.tar.gz";
    hash = mozjsArchiveWindowsHash;
  };

  ortArchiveWindows = pkgs.fetchurl {
    url = "https://github.com/microsoft/onnxruntime/releases/download/v${ortVersion}/onnxruntime-win-x64-${ortVersion}.zip";
    hash = ortArchiveWindowsHash;
  };

  windowsCross = pkgs.callPackage ../packages/windows-cross.nix { };

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
      pkgs.callPackage ../packages/release.nix (
        releaseCommonArgs
        // {
          platform = "windows";
          inherit minne-pkg-windows ortArchiveWindows;
          targetTriple = windowsTarget;
        }
      )
    else
      null;
in
{
  inherit
    xwinCargoCache
    minne-pkg-windows
    minne-release-windows
    ;
}
