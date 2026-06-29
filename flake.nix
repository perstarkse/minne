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
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    let
      versions = import ./nix/versions.nix;
    in
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.treefmt-nix.flakeModule
        ./nix/modules/context.nix
        ./nix/modules/packages.nix
        ./nix/modules/checks.nix
        ./nix/modules/formatter.nix
        ./nix/modules/dev-shell.nix
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      _module.args = {
        inherit versions;
      };

      flake = {
        nixosModules = rec {
          minne = import ./nix/nixos/minne.nix { inherit (inputs) self; };
          default = minne;
        };

        lib = {
          inherit (versions) ortVersion rustVersion minneVersion;
        };
      };
    };
}
