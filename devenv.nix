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
    pkgs.cargo-dist
    pkgs.cargo-xwin
    pkgs.clang
    pkgs.onnxruntime
    pkgs.cargo-watch
    pkgs.tailwindcss_4
  ];

  languages.rust = {
    enable = true;
    components = ["rustc" "clippy" "rustfmt" "cargo" "rust-analyzer"];
    channel = "nightly";
    targets = ["x86_64-unknown-linux-gnu" "x86_64-pc-windows-msvc"];
    mold.enable = true;
  };

  env = {
    ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime.so";
  };

  processes = {
    surreal_db.exec = "docker run --rm --pull always -p 8000:8000 --net=host --user $(id -u) -v $(pwd)/database:/database surrealdb/surrealdb:latest-dev start rocksdb:/database/database.db --user root_user --pass root_password";
    server.exec = "cargo watch -x 'run --bin main'";
    tailwind.exec = "cd html-router && npm run tailwind";
  };
}
