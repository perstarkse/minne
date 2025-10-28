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
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        minne-pkg = craneLib.buildPackage {
          src = craneLib.cleanCargoSource ./.;
          pname = "minne";
          version = "0.1.0";

          doCheck = false;

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.rustfmt
            pkgs.makeWrapper
          ];

          buildInputs = [
            pkgs.openssl
            pkgs.chromium
            pkgs.onnxruntime
          ];

          ORT_STRATEGY = "system";
          ORT_LIB_LOCATION = "${pkgs.onnxruntime}/lib";
          ORT_SKIP_DOWNLOAD = "1";

          postInstall = ''
            wrapProgram $out/bin/main \
              --set CHROME ${pkgs.chromium}/bin/chromium \
              --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.so
            if [ -f $out/bin/worker ]; then
              wrapProgram $out/bin/worker \
                --set CHROME ${pkgs.chromium}/bin/chromium \
                --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.so
            fi
            if [ -f $out/bin/server]; then
              wrapProgram $out/bin/server\
                --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.so
            fi
          '';
        };
      in {
        packages = {
          default = self.packages.${system}.minne-pkg;
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
          default = self.apps.${system}.main;
        };
      }
    );
}
