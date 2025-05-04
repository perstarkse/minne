# flake.nix
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
            # outputHashes = { ... }; # Only if other git dependencies still exist
          };

          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.rustfmt
            pkgs.makeWrapper # For the postInstall hook
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
          name = "minne"; # Docker image repository name
          tag = minne-pkg.version; # Use the package version as the image tag

          # Include the runtime closure of our minne package in the image layers
          # Also add bash for easier debugging inside the container
          contents = [minne-pkg pkgs.bashInteractive];

          # Configure the runtime behavior of the Docker image
          config = {
            # Set the default command to run when the container starts
            # Assumes 'main' is your primary entrypoint
            Cmd = ["${minne-pkg}/bin/main"];

            # Add other Docker config as needed:
            # ExposedPorts = { "8080/tcp" = {}; }; # Example port exposure
            WorkingDir = "/data"; # Example working directory
            # Volumes = { "/data" = {}; };     # Example volume mount point
            Env = [
              # The wrapper should set CHROME_BIN, but you can add other env vars
              "PATH=${pkgs.lib.makeBinPath [minne-pkg pkgs.coreutils]}" # Ensure coreutils are in PATH
              "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" # Common requirement
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
          # Default app for 'nix run .'
          default = self.apps.${system}.main;
        };

        # # Development Shell: Accessible via 'nix develop'
        # devShells.default = pkgs.mkShell {
        #   # Use inputs from the main package derivation
        #   inputsFrom = [minne-pkg];
        #   # Add development tools
        #   nativeBuildInputs = minne-pkg.nativeBuildInputs;
        #   buildInputs =
        #     minne-pkg.buildInputs
        #     ++ [
        #       pkgs.cargo
        #       pkgs.rustc
        #       pkgs.clippy # Add other dev tools as needed
        #     ];
        #   # Add shell hooks or env vars if needed
        #   # shellHook = ''
        #   #   export MY_DEV_VAR="hello"
        #   # '';
        # };
      }
    );
}
