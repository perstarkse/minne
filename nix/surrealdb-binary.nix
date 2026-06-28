# Pinned SurrealDB release binary (avoids building nixpkgs surrealdb from source).
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  openssl,
  rocksdb,
}:
stdenv.mkDerivation {
  pname = "surrealdb";
  version = "2.6.5";

  src = fetchurl {
    url = "https://github.com/surrealdb/surrealdb/releases/download/v2.6.5/surreal-v2.6.5.linux-amd64.tgz";
    hash = "sha256-kp1z9GxPtZ8jeBDm/m2lTBdWBk8+2NfR9qlw6P3zj7A=";
  };

  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [
    openssl
    rocksdb
    stdenv.cc.cc.lib
  ];

  sourceRoot = ".";

  installPhase = ''
    runHook preInstall
    install -Dm755 surreal $out/bin/surreal
    runHook postInstall
  '';

  meta = {
    description = "SurrealDB prebuilt binary pinned for local dev and VM smoke tests";
    homepage = "https://surrealdb.com/";
    license = lib.licenses.bsl11;
    platforms = [ "x86_64-linux" ];
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
  };
}
