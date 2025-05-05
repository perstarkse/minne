{
  description = "Minne application flake";

  # Specify the inputs for our flake (nixpkgs for packages, flake-utils for convenience)
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # Or pin to a specific release/commit
    flake-utils.url = "github:numtide/flake-utils";
  };

  # Define the outputs of our flake (packages, apps, shells, etc.)
  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
  # Use flake-utils to generate outputs for common systems (x86_64-linux, aarch64-linux, x86_64-darwin)
    flake-utils.lib.eachDefaultSystem (
      system: let
        # Get the package set for the current system
        pkgs = nixpkgs.legacyPackages.${system};

        # --- Minne Package Definition ---
        # This is your core rust application build
        minne-pkg = pkgs.rustPlatform.buildRustPackage {
          pname = "minne";
          version = "0.1.0"; # Consider fetching this from Cargo.toml later

          # Source is the flake's root directory
          src = self;

          # Assuming you switched to crates.io headless_chrome
          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.rustfmt
            # pkgs.makeWrapper # For the postInstall hook
          ];
          buildInputs = [
            pkgs.openssl
            pkgs.chromium # Runtime dependency for the browser
          ];

          # Wrap the actual executables to provide CHROME_BIN at runtime
          postInstall = let
            # Define path to nix-provided chromium executable
            chromium_executable = "${pkgs.chromium}/bin/chromium";
          in ''
            echo "Wrapping binaries in postInstall hook..."
            ls -l $out/bin # Add this line to debug which binaries are present
            wrapProgram $out/bin/main \
              --set CHROME_BIN "${chromium_executable}"
            wrapProgram $out/bin/worker \
              --set CHROME_BIN "${chromium_executable}"
            echo "Finished wrapping."
          '';

          meta = with pkgs.lib; {
            description = "Minne Application";
            license = licenses.mit; # Adjust if needed
          };
        };

        # --- Docker Image Definition (using dockerTools) ---
        minne-docker-image = pkgs.dockerTools.buildImage {
          name = "minne";
          tag = minne-pkg.version;

          # Create an environment containing our packages
          # and copy its contents to the image's root filesystem.
          copyToRoot = pkgs.buildEnv {
            name = "minne-env"; # Name for the build environment derivation
            paths = [
              minne-pkg # Include our compiled Rust application
              pkgs.bashInteractive # Include bash for debugging/interaction
              pkgs.coreutils # Often useful to have basic utils like ls, cat etc.
              pkgs.cacert # Include CA certificates for TLS/SSL
            ];
            # Optional: Add postBuild hook for the buildEnv if needed
            # postBuild = '' ... '';
          };

          # Configure the runtime behavior of the Docker image
          config = {
            # Cmd can now likely refer to the binary directly in /bin
            # (buildEnv symlinks the 'main' binary into the profile's bin)
            Cmd = ["/bin/main"];

            # ExposedPorts = { "8080/tcp" = {}; };
            WorkingDir = "/data";
            # Volumes = { "/data" = {}; };

            # PATH might not need explicit setting if things are in /bin,
            # but setting explicitly can be safer. buildEnv adds its bin path automatically.
            Env = [
              # SSL_CERT_FILE is often essential for HTTPS requests
              "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
            ];
          };
        };
      in {
        # --- Flake Outputs ---

        # Packages: Accessible via 'nix build .#minne' or '.#minne-docker'
        packages = {
          minne = minne-pkg;
          minne-docker = minne-docker-image;
          # Default package for 'nix build .'
          default = self.packages.${system}.minne;
        };

        # Apps: Accessible via 'nix run .#main' or '.#worker'
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
          # Default app for 'nix run .'
          default = self.apps.${system}.main;
        };
      }
    );
}
