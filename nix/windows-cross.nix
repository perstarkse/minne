# Offline MSVC CRT + Windows SDK for cross-compiling to x86_64-pc-windows-msvc.
{
  lib,
  stdenv,
  xwin,
  cacert,
  writeText,
  writeShellScript,
}:
let
  cmakeOverride = writeText "override.cmake" ''
    # macOS paths usually start with /Users/*. Unfortunately, clang-cl interprets
    # paths starting with /U as macro undefines, so we need to put a -- before the
    # input file path to force it to be treated as a path.
    string(REPLACE "-c <SOURCE>" "-c -- <SOURCE>" CMAKE_C_COMPILE_OBJECT "''${CMAKE_C_COMPILE_OBJECT}")
    string(REPLACE "-c <SOURCE>" "-c -- <SOURCE>" CMAKE_CXX_COMPILE_OBJECT "''${CMAKE_CXX_COMPILE_OBJECT}")
    string(REPLACE "/D" "-D" CMAKE_RC_FLAGS "''${CMAKE_RC_FLAGS_INIT}")
    string(REPLACE "/D" "-D" CMAKE_RC_FLAGS_DEBUG "''${CMAKE_RC_FLAGS_DEBUG_INIT}")
    if(NOT CMAKE_HOST_WIN32)
      set(CMAKE_NINJA_CMCLDEPS_RC 0)
    endif()
  '';

  cmakeToolchain = writeText "x86_64-pc-windows-msvc-toolchain.cmake" ''
    set(CMAKE_SYSTEM_NAME Windows)
    set(CMAKE_SYSTEM_PROCESSOR AMD64)
    set(CMAKE_C_COMPILER clang-cl CACHE FILEPATH "")
    set(CMAKE_CXX_COMPILER clang-cl CACHE FILEPATH "")
    set(CMAKE_AR llvm-lib)
    set(CMAKE_LINKER lld-link CACHE FILEPATH "")
    set(CMAKE_MSVC_RUNTIME_LIBRARY CACHE STRING "MultiThreadedDLL")

    # Paths relative to this file (cmake/clang-cl/) so the FOD output stays store-path-free.
    set(_XWIN_ROOT "''${CMAKE_CURRENT_LIST_DIR}/../..")
    set(_CRT "''${_XWIN_ROOT}/xwin/crt")
    set(_SDK "''${_XWIN_ROOT}/xwin/sdk")

    set(COMPILE_FLAGS
        --target=x86_64-pc-windows-msvc
        -Wno-unused-command-line-argument
        -fuse-ld=lld-link
        /imsvc ''${_CRT}/include
        /imsvc ''${_SDK}/include/ucrt
        /imsvc ''${_SDK}/include/um
        /imsvc ''${_SDK}/include/shared
        /imsvc ''${_SDK}/include/winrt)
    set(LINK_FLAGS
        /manifest:no
        -libpath:"''${_CRT}/lib/x86_64"
        -libpath:"''${_SDK}/lib/um/x86_64"
        -libpath:"''${_SDK}/lib/ucrt/x86_64")
    string(REPLACE ";" " " COMPILE_FLAGS "''${COMPILE_FLAGS}")
    set(_CMAKE_C_FLAGS_INITIAL "''${CMAKE_C_FLAGS}" CACHE STRING "")
    set(CMAKE_C_FLAGS "''${_CMAKE_C_FLAGS_INITIAL} ''${COMPILE_FLAGS}" CACHE STRING "" FORCE)
    set(_CMAKE_CXX_FLAGS_INITIAL "''${CMAKE_CXX_FLAGS}" CACHE STRING "")
    set(CMAKE_CXX_FLAGS "''${_CMAKE_CXX_FLAGS_INITIAL} ''${COMPILE_FLAGS} /EHsc" CACHE STRING "" FORCE)
    string(REPLACE ";" " " LINK_FLAGS "''${LINK_FLAGS}")
    set(_CMAKE_EXE_LINKER_FLAGS_INITIAL "''${CMAKE_EXE_LINKER_FLAGS}" CACHE STRING "")
    set(CMAKE_EXE_LINKER_FLAGS "''${_CMAKE_EXE_LINKER_FLAGS_INITIAL} ''${LINK_FLAGS}" CACHE STRING "" FORCE)
    set(_CMAKE_MODULE_LINKER_FLAGS_INITIAL "''${CMAKE_MODULE_LINKER_FLAGS}" CACHE STRING "")
    set(CMAKE_MODULE_LINKER_FLAGS "''${_CMAKE_MODULE_LINKER_FLAGS_INITIAL} ''${LINK_FLAGS}" CACHE STRING "" FORCE)
    set(_CMAKE_SHARED_LINKER_FLAGS_INITIAL "''${CMAKE_SHARED_LINKER_FLAGS}" CACHE STRING "")
    set(CMAKE_SHARED_LINKER_FLAGS "''${_CMAKE_SHARED_LINKER_FLAGS_INITIAL} ''${LINK_FLAGS}" CACHE STRING "" FORCE)
    set(CMAKE_C_STANDARD_LIBRARIES "" CACHE STRING "" FORCE)
    set(CMAKE_CXX_STANDARD_LIBRARIES "" CACHE STRING "" FORCE)
    set(CMAKE_TRY_COMPILE_CONFIGURATION Release)
    set(CMAKE_USER_MAKE_RULES_OVERRIDE "''${CMAKE_CURRENT_LIST_DIR}/override.cmake")
  '';

  clangClWrapper = writeShellScript "clang-cl-msvc-link-wrapper" ''
    # Route mozangle DLL link invocations (clang-cl + /LD + .obj) to lld-link.
    set -euo pipefail

    real_clang_cl="''${REAL_CLANG_CL:-clang-cl}"
    real_lld_link="''${REAL_LLD_LINK:-lld-link}"

    objs=()
    libs=()
    fe=""
    def=""
    has_ld=false

    for arg in "$@"; do
      case "$arg" in
        *.obj|*.o) objs+=("$arg") ;;
        *.lib) libs+=("$arg") ;;
        /LD|/Ld) has_ld=true ;;
        /Fe*) fe="''${arg#/Fe}" ;;
        /DEF:*) def="$arg" ;;
      esac
    done

    if [[ "$has_ld" == true && ''${#objs[@]} -gt 0 ]]; then
      lld_args=()
      for o in "''${objs[@]}"; do
        lld_args+=("$o")
      done
      for l in "''${libs[@]}"; do
        lld_args+=("$l")
      done
      lld_args+=("/DLL")
      if [[ -n "$fe" ]]; then
        lld_args+=("/OUT:$fe")
      fi
      if [[ -n "$def" ]]; then
        lld_args+=("$def")
      fi
      exec "$real_lld_link" "''${lld_args[@]}"
    fi

    exec "$real_clang_cl" "$@"
  '';
in
{
  inherit clangClWrapper;

  xwinCargoCache = stdenv.mkDerivation {
    pname = "cargo-xwin-cache";
    version = "17.2";

    nativeBuildInputs = [xwin cacert];

    preferLocalBuild = true;
    allowSubstitutes = false;
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = "sha256-eYRtNXLttjlC1l+9mMoeXjmGwSC86hhY3on1IN4dym8=";

    buildCommand = ''
      set -eo pipefail
      export XWIN_ACCEPT_LICENSE=true
      export SSL_CERT_FILE="${cacert}/etc/ssl/certs/ca-certificates.crt"
      export SSL_CERT_DIR="${cacert}/etc/ssl/certs"
      export NIX_SSL_CERT_FILE="${cacert}/etc/ssl/certs/ca-certificates.crt"

      dl="$TMPDIR/xwin-dl"
      splat="$TMPDIR/splat"
      mkdir -p "$dl" "$splat"

      ${xwin}/bin/xwin \
        --manifest-version 17 \
        --arch x86_64 \
        --cache-dir "$dl" \
        download
      ${xwin}/bin/xwin \
        --manifest-version 17 \
        --arch x86_64 \
        --cache-dir "$dl" \
        unpack
      ${xwin}/bin/xwin \
        --manifest-version 17 \
        --arch x86_64 \
        --cache-dir "$dl" \
        splat --output "$splat"

      mkdir -p "$out/xwin" "$out/cmake/clang-cl"
      cp -a "$splat"/. "$out/xwin/"
      echo -n "x86_64" > "$out/xwin/DONE"

      cp ${cmakeToolchain} "$out/cmake/clang-cl/x86_64-pc-windows-msvc-toolchain.cmake"
      cp ${cmakeOverride} "$out/cmake/clang-cl/override.cmake"

      # lld-link expects Dbghelp.lib; xwin ships dbghelp.lib (case-sensitive FS).
      for dir in "$out/xwin/crt/lib/x86_64" "$out/xwin/sdk/lib/um/x86_64"; do
        if [ -f "$dir/dbghelp.lib" ] && [ ! -e "$dir/Dbghelp.lib" ]; then
          ln -sf dbghelp.lib "$dir/Dbghelp.lib"
        fi
      done
    '';
  };
}
