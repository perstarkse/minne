{
  fenix,
  system,
  rustVersion,
  manifestSha256,
}:
let
  components = [
    "rustc"
    "cargo"
    "clippy"
    "rustfmt"
    "rust-analyzer"
  ];
  targets = [
    "x86_64-unknown-linux-gnu"
    "x86_64-pc-windows-msvc"
  ];
  windowsTarget = "x86_64-pc-windows-msvc";

  toolchainFile = builtins.toFile "rust-toolchain.toml" ''
    [toolchain]
    channel = "${rustVersion}"
    components = ${builtins.toJSON components}
    targets = ${builtins.toJSON targets}
  '';

  fenixPkgs = fenix.packages.${system};
  toolchain = fenixPkgs.fromToolchainFile {
    file = toolchainFile;
    sha256 = manifestSha256;
  };
in
{
  inherit rustVersion toolchain windowsTarget;

  mkCraneLib = _: craneLib: craneLib.overrideToolchain (_: toolchain);
}
