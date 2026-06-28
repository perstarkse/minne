{
  description = "Minne application flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
    git-hooks.url = "github:cachix/git-hooks.nix";
    git-hooks.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    let
      ortVersion = "1.23.2";
      toolchainFile = ./rust-toolchain.toml;
      rustVersion = (builtins.fromTOML (builtins.readFile toolchainFile)).toolchain.channel;
    in
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        ./nix/context.nix
        ./nix/package.nix
        ./nix/checks.nix
        ./nix/dev-shell.nix
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      _module.args = {
        inherit ortVersion toolchainFile rustVersion;
      };

      flake = {
        lib = {
          inherit ortVersion rustVersion;
        };
      };
    };
}
