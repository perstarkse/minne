{
  lib,
  stdenv,
  platform ? (
    if stdenv.isLinux then
      "linux"
    else if stdenv.isDarwin then
      "darwin"
    else
      "windows"
  ),
  patchelf ? null,
  xz,
  unzip ? null,
  zip ? null,
  minne-pkg ? null,
  minne-pkg-windows ? null,
  onnxruntime ? null,
  ortArchiveWindows ? null,
  libglvnd ? null,
  fontconfig,
  freetype,
  openssl,
  zlib,
  bzip2,
  libpng,
  brotli,
  expat,
  gcc ? null,
  glibc ? null,
  minneVersion,
  targetTriple,
  srcRoot,
}:
let
  archiveName = "main-${targetTriple}";
  binaries = [
    "main"
    "server"
    "worker"
  ];

  copyBinary = ''
    if [ -f "${minne-pkg}/bin/.$b-wrapped" ]; then
      cp "${minne-pkg}/bin/.$b-wrapped" "$exe"
    elif [ -f "${minne-pkg}/bin/$b" ]; then
      cp "${minne-pkg}/bin/$b" "$exe"
    else
      echo "missing binary: $b" >&2
      exit 1
    fi
  '';

  copyDocs = ''
    cp ${srcRoot}/README.md ${srcRoot}/LICENSE ${srcRoot}/CHANGELOG.md "$root/"
  '';

  mkLinux =
    let
      isAarch64 = lib.hasSuffix "aarch64-unknown-linux-gnu" targetTriple;
      dynamicLinker = if isAarch64 then "ld-linux-aarch64.so.1" else "ld-linux-x86-64.so.2";

      copySharedLibs = pkg: ''
        if [ -d "${pkg}/lib" ]; then
          find -L "${pkg}/lib" -maxdepth 1 \( -name '*.so' -o -name '*.so.*' \) -exec cp -L {} "$root/lib/" \;
        fi
      '';
    in
    stdenv.mkDerivation {
      pname = "main";
      version = minneVersion;

      nativeBuildInputs = [
        patchelf
        xz
      ];

      dontUnpack = true;
      dontStrip = true;
      dontPatchELF = true;

      installPhase = ''
        runHook preInstall

        root="$TMPDIR/root/${archiveName}"
        mkdir -p "$root/lib"

        for b in ${lib.concatStringsSep " " binaries}; do
          exe="$root/.$b-bin"
          ${copyBinary}
          chmod u+w "$exe"
          patchelf --set-interpreter 'lib/${dynamicLinker}' "$exe"
          patchelf --set-rpath '$ORIGIN/lib' "$exe"
          printf '%s\n' \
            '#!/bin/sh' \
            'set -eu' \
            'DIR="$(CDPATH= cd "$(dirname "$0")" && pwd)"' \
            'export LD_LIBRARY_PATH="$DIR/lib"' \
            'export ORT_DYLIB_PATH="$DIR/lib/libonnxruntime.so"' \
            'exec "$DIR/.'"$b"'-bin" "$@"' \
            > "$root/$b"
          chmod +x "$root/$b"
        done

        cp -L ${onnxruntime}/lib/libonnxruntime.so* "$root/lib/"
        ${copySharedLibs libglvnd}
        ${copySharedLibs fontconfig.lib}
        ${copySharedLibs freetype}
        ${copySharedLibs openssl.out}
        ${copySharedLibs zlib}
        ${copySharedLibs bzip2}
        ${copySharedLibs libpng}
        ${copySharedLibs brotli}
        ${copySharedLibs expat}
        ${copySharedLibs gcc.cc.lib}

        # Bundle glibc so binaries run on older distros (e.g. Ubuntu 22.04).
        cp -L ${glibc}/lib/${dynamicLinker} "$root/lib/"
        for lib in libc.so.6 libm.so.6 libpthread.so.0 libdl.so.2 librt.so.1 libresolv.so.2; do
          if [ -f "${glibc}/lib/$lib" ]; then
            cp -L "${glibc}/lib/$lib" "$root/lib/"
          fi
        done

        ${copyDocs}

        mkdir -p "$out"
        tar -cJf "$out/${archiveName}.tar.xz" -C "$TMPDIR/root" "${archiveName}"

        runHook postInstall
      '';

      meta = {
        description = "Minne release archive for ${targetTriple}";
        platforms = lib.platforms.linux;
      };
    };

  mkDarwin =
    let
      copyDylibs = pkg: ''
        if [ -d "${pkg}/lib" ]; then
          find -L "${pkg}/lib" -maxdepth 1 \( -name '*.dylib' -o -name '*.so' \) -exec cp -L {} "$root/lib/" \;
        fi
      '';
    in
    stdenv.mkDerivation {
      pname = "main";
      version = minneVersion;

      nativeBuildInputs = [ xz ];

      dontUnpack = true;
      dontStrip = true;
      dontFixDarwinDylibs = true;

      installPhase = ''
        runHook preInstall

        root="$TMPDIR/root/${archiveName}"
        mkdir -p "$root/lib"

        for b in ${lib.concatStringsSep " " binaries}; do
          exe="$root/.$b-bin"
          ${copyBinary}
          chmod +x "$exe"
          printf '%s\n' \
            '#!/bin/sh' \
            'set -eu' \
            'DIR="$(CDPATH= cd "$(dirname "$0")" && pwd)"' \
            'export DYLD_LIBRARY_PATH="$DIR/lib''${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"' \
            'exec "$DIR/.'"$b"'-bin" "$@"' \
            > "$root/$b"
          chmod +x "$root/$b"
        done

        cp -L ${onnxruntime}/lib/libonnxruntime*.dylib "$root/lib/" 2>/dev/null || true
        if [ ! -f "$root/lib/libonnxruntime.dylib" ]; then
          ort="$(find -L "$root/lib" -maxdepth 1 -name 'libonnxruntime*.dylib' | head -n1)"
          if [ -n "$ort" ]; then
            cp -L "$ort" "$root/lib/libonnxruntime.dylib"
          else
            echo "missing libonnxruntime.dylib" >&2
            exit 1
          fi
        fi

        ${copyDylibs fontconfig.lib}
        ${copyDylibs freetype}
        ${copyDylibs openssl.out}
        ${copyDylibs zlib}
        ${copyDylibs bzip2}
        ${copyDylibs libpng}
        ${copyDylibs brotli}
        ${copyDylibs expat}

        ${copyDocs}

        mkdir -p "$out"
        tar -cJf "$out/${archiveName}.tar.xz" -C "$TMPDIR/root" "${archiveName}"

        runHook postInstall
      '';

      meta = {
        description = "Minne release archive for ${targetTriple}";
        platforms = lib.platforms.darwin;
      };
    };

  mkWindows = stdenv.mkDerivation {
    pname = "main";
    version = minneVersion;

    nativeBuildInputs = [
      unzip
      zip
    ];

    dontUnpack = true;
    dontStrip = true;

    installPhase = ''
      runHook preInstall

      root="$TMPDIR/root"
      mkdir -p "$root/lib"

      for b in ${lib.concatStringsSep " " binaries}; do
        if [ ! -f "${minne-pkg-windows}/bin/$b.exe" ]; then
          echo "missing binary: $b.exe" >&2
          exit 1
        fi
        cp "${minne-pkg-windows}/bin/$b.exe" "$root/$b.exe"
      done

      unzip -q ${ortArchiveWindows} -d "$TMPDIR/ort"
      dll="$(find "$TMPDIR/ort" -name onnxruntime.dll -print -quit)"
      if [ -z "$dll" ]; then
        echo "missing onnxruntime.dll in ORT archive" >&2
        exit 1
      fi
      cp "$dll" "$root/lib/onnxruntime.dll"

      ${copyDocs}

      mkdir -p "$out"
      (cd "$root" && zip -qr "$out/${archiveName}.zip" .)

      runHook postInstall
    '';

    meta = {
      description = "Minne release archive for ${targetTriple}";
      platforms = [ "x86_64-linux" ];
    };
  };
in
if platform == "linux" then
  mkLinux
else if platform == "darwin" then
  mkDarwin
else if platform == "windows" then
  mkWindows
else
  throw "minne-release: unknown platform ${platform}"
