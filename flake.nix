{
  description = "Minne application flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};

        # --- Minne Package Definition ---
        minne-pkg = pkgs.rustPlatform.buildRustPackage {
          pname = "minne";
          version = "0.1.0";

          src = self;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # Skip tests due to testing fs operations
          doCheck = false;

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.rustfmt
            pkgs.makeWrapper # For the postInstall hook
          ];
          buildInputs = [
            pkgs.openssl
            pkgs.chromium # Runtime dependency for the browser
          ];

          # Wrap the actual executables to provide CHROME at runtime
          postInstall = let
            chromium_executable = "${pkgs.chromium}/bin/chromium";
          in ''
            wrapProgram $out/bin/main \
              --set CHROME "${chromium_executable}"
            wrapProgram $out/bin/worker \
              --set CHROME "${chromium_executable}"
          '';

          meta = with pkgs.lib; {
            description = "Minne Application";
            license = licenses.mit;
          };
        };
      in {
        packages = {
          minne = minne-pkg;
          default = self.packages.${system}.minne;
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
