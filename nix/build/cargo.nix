{
  inputs,
  pkgs,
  system,
  src,
  ortVersion,
  versions,
}:
let
  inherit (pkgs) lib;
  inherit (inputs) crane fenix;
  inherit (versions)
    minneVersion
    mozjsRelease
    mozjsHashes
    rustVersion
    rustManifestSha256
    ;

  rustToolchain = import ../packages/rust-toolchain.nix {
    inherit fenix system rustVersion;
    manifestSha256 = rustManifestSha256;
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

  mozjsTarget =
    {
      "x86_64-linux" = "x86_64-unknown-linux-gnu";
      "aarch64-linux" = "aarch64-unknown-linux-gnu";
      "aarch64-darwin" = "aarch64-apple-darwin";
      "x86_64-darwin" = "x86_64-apple-darwin";
    }
    .${system} or (throw "mozjs prebuilt archive not configured for system ${system}");

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
      pkgs.callPackage ../packages/release.nix (
        releaseCommonArgs
        // {
          platform = "linux";
          inherit minne-pkg;
        }
      )
    else if pkgs.stdenv.isDarwin then
      pkgs.callPackage ../packages/release.nix (
        releaseCommonArgs
        // {
          platform = "darwin";
          inherit minne-pkg;
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
in
{
  inherit
    lib
    craneLib
    rustToolchain
    libExt
    linuxRuntimeLibs
    devGraphicsLibs
    commonArgs
    minneVersion
    minne-pkg
    minne-release
    releaseCommonArgs
    dockerImage
    ;
}
