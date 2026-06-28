# Evaluates shared build context once per system for downstream flake-parts modules.
{
  inputs,
  ortVersion,
  toolchainFile,
  ...
}:
{
  perSystem =
    { system, ... }:
    let
      pkgs = import inputs.nixpkgs {
        inherit system;
        config = {
          allowUnfree = true;
          permittedInsecurePackages = [ "minio-2025-10-15T17-29-55Z" ];
        };
      };
    in
    {
      _module.args.pkgs = pkgs;
      _module.args.minneCtx = import ./minne-lib.nix {
        inherit
          inputs
          pkgs
          system
          ortVersion
          toolchainFile
          ;
        src = inputs.self;
      };
    };
}
