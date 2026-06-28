# Shared Rust toolchain for dev shell, crane builds, and cross-compilation.
{
  fenix,
  system,
  toolchainFile,
  # Manifest hash for https://static.rust-lang.org/dist/channel-rust-<version>.toml
  manifestSha256 ? "sha256-SDu4snEWjuZU475PERvu+iO50Mi39KVjqCeJeNvpguU=",
}:
let
  fenixPkgs = fenix.packages.${system};
  toolchain = fenixPkgs.fromToolchainFile {
    file = toolchainFile;
    sha256 = manifestSha256;
  };
  parsed = builtins.fromTOML (builtins.readFile toolchainFile);
  rustVersion = parsed.toolchain.channel;
  windowsTarget = "x86_64-pc-windows-msvc";
in
{
  inherit rustVersion toolchain windowsTarget;

  mkCraneLib = pkgs: craneLib: craneLib.overrideToolchain (_: toolchain);
}
