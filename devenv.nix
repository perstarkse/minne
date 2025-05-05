{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  cachix.enable = false;

  packages = [
    pkgs.openssl
    pkgs.nodejs
    pkgs.vscode-langservers-extracted
  ];

  languages.rust = {
    enable = true;
    components = ["rustc" "clippy" "rustfmt" "cargo" "rust-analyzer"];
    mold.enable = true;
  };

  processes = {
    surreal_db.exec = "docker run --rm --pull always -p 8000:8000 --net=host --user $(id -u) -v $(pwd)/database:/database surrealdb/surrealdb:latest-dev start rocksdb:/database/database.db --user root_user --pass root_password";
  };
}
