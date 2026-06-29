{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  openssl,
  rocksdb,
  surrealdbVersion ? (import ../../versions.nix).surrealdbVersion,
  surrealdbBinaryHash ? (import ../../versions.nix).surrealdbBinaryHash,
}:
stdenv.mkDerivation {
  pname = "surrealdb";
  version = surrealdbVersion;

  src = fetchurl {
    url = "https://github.com/surrealdb/surrealdb/releases/download/v${surrealdbVersion}/surreal-v${surrealdbVersion}.linux-amd64.tgz";
    hash = surrealdbBinaryHash;
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
