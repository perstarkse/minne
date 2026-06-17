{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:
let
  ortVersion = lib.removeSuffix "\n" (builtins.readFile "${toString ./.}/ort-version");
  _ortVersionCheck =
    if pkgs.onnxruntime.version == ortVersion
    then null
    else
      throw "pkgs.onnxruntime.version (${pkgs.onnxruntime.version}) must match ort-version (${ortVersion})";
in {
  devenv.warnOnNewVersion = false;

  cachix.enable = false;

  packages = [
    pkgs.openssl
    pkgs.nodejs
    pkgs.watchman
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
    channel = "stable";
    version = "1.91.1";
    components = ["rustc" "clippy" "rustfmt" "cargo" "rust-analyzer"];
    targets = ["x86_64-unknown-linux-gnu" "x86_64-pc-windows-msvc"];
    mold.enable = true;
  };

  env = {
    ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime.so";
    S3_ENDPOINT = "http://127.0.0.1:19000";
    S3_BUCKET = "minne-tests";
    MINNE_TEST_S3_ENDPOINT = "http://127.0.0.1:19000";
    MINNE_TEST_S3_BUCKET = "minne-tests";
  };

  services.minio = {
    enable = true;
    listenAddress = "127.0.0.1:19000";
    consoleAddress = "127.0.0.1:19001";
    buckets = ["minne-tests"];
    accessKey = "minioadmin";
    secretKey = "minioadmin";
    region = "us-east-1";
  };

  processes = {
    surreal_db.exec = "docker run --rm --pull always -p 8000:8000 --net=host --user $(id -u) -v $(pwd)/database:/database surrealdb/surrealdb:v2.6.5-dev start rocksdb:/database/database.db --user root_user --pass root_password";
    tailwind.exec = "tailwindcss --cwd html-router -i app.css -o assets/style.css --watch=always";
  };
}
