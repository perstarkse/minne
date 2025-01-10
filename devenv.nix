{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  # https://devenv.sh/basics/
  # env.GREET = "devenv";
  cachix.enable = false;

  # https://devenv.sh/packages/
  packages = [
    pkgs.openssl
    pkgs.nodejs
    pkgs.vscode-langservers-extracted
  ];

  # https://devenv.sh/languages/
  languages.rust.enable = true;

  # https://devenv.sh/services/
  # services.postgres.enable = true;

  # https://devenv.sh/scripts/
  # scripts.hello.exec = ''
  #   echo hello from $GREET
  # '';

  # enterShell = ''
  #   hello
  #   git --version
  # '';

  # https://devenv.sh/processes/
  processes = {
    surreal_db.exec = "docker run --rm --pull always -p 8000:8000 --net=host --user $(id -u) -v $(pwd)/database:/database surrealdb/surrealdb:latest-dev start rocksdb:/database/database.db --user root_user --pass root_password";
  };

  # services = {
  #   rabbitmq = {
  #     enable =
  #       true;
  #     # plugins = ["tracing"];
  #   };
  # };
  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/tests/
  # enterTest = ''
  #   echo "Running tests"
  #   git --version | grep --color=auto "${pkgs.git.version}"
  # '';

  # https://devenv.sh/pre-commit-hooks/
  # pre-commit.hooks.shellcheck.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
