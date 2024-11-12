{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
    devenv.url = "github:cachix/devenv";
    devenv.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };

  outputs = {
    self,
    nixpkgs,
    devenv,
    systems,
    ...
  } @ inputs: let
    forEachSystem = nixpkgs.lib.genAttrs (import systems);
  in {
    packages = forEachSystem (system: {
      devenv-up = self.devShells.${system}.default.config.procfileScript;
    });

    devShells =
      forEachSystem
      (system: let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        default = devenv.lib.mkShell {
          inherit inputs pkgs;
          modules = [
            {
              # https://devenv.sh/reference/options/
              enterShell = ''
                echo "Welcome to zettle_db project"
                echo "----------------------------"
                echo "run devenv up -d to start and monitor services"
              '';

              packages = [
              ];

              languages.rust.enable = true;

              processes = {
                surreal_db.exec = "docker run --rm --pull always -p 8000:8000 --user $(id -u) -v $(pwd)/database:/database surrealdb/surrealdb:latest-dev start rocksdb:/database/database.db --user root_user --pass root_password";
              };

              services = {
                rabbitmq = {
                  enable = true;
                  # plugins = ["tracing"];
                };
              };
            }
          ];
        };
      });
  };
}
