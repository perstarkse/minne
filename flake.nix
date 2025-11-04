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
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      lib = pkgs.lib;
      craneLib = crane.mkLib pkgs;
      libExt =
        if pkgs.stdenv.isDarwin
        then "dylib"
        else "so";
      minne-pkg = craneLib.buildPackage {
        src = lib.cleanSourceWith {
          src = ./.;
          filter = let
            extraPaths = [
              (toString ./Cargo.lock)
              (toString ./common/migrations)
              (toString ./common/schemas)
              (toString ./html-router/templates)
              (toString ./html-router/assets)
            ];
          in
            path: type: let
              p = toString path;
            in
              craneLib.filterCargoSources path type
              || lib.any (x: lib.hasPrefix x p) extraPaths;
        };

        pname = "minne";
        version = "0.2.6";
        doCheck = false;

        nativeBuildInputs = [pkgs.pkg-config pkgs.rustfmt pkgs.makeWrapper];
        buildInputs = [pkgs.openssl pkgs.chromium pkgs.onnxruntime];

        postInstall = ''
          wrapProgram $out/bin/main \
            --set CHROME ${pkgs.chromium}/bin/chromium \
            --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
          for b in worker server; do
            if [ -x "$out/bin/$b" ]; then
              wrapProgram $out/bin/$b \
                --set CHROME ${pkgs.chromium}/bin/chromium \
                --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.${libExt}
            fi
          done
        '';
      };
    in {
      packages = {
        minne-pkg = minne-pkg;
        default = minne-pkg;
      };
      apps = {
        main = flake-utils.lib.mkApp {
          drv = minne-pkg;
          name = "main";
        };
        worker = flake-utils.lib.mkApp {
          drv = minne-pkg;
          name = "worker";
        };
        server = flake-utils.lib.mkApp {
          drv = minne-pkg;
          name = "server";
        };
        default = flake-utils.lib.mkApp {
          drv = minne-pkg;
          name = "main";
        };
      };
    });
}
