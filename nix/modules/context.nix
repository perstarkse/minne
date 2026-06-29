{
  inputs,
  versions,
  ...
}:
{
  perSystem =
    { system, ... }:
    let
      inherit (versions) ortVersion;

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
      _module.args.minneCtx = import ../build/default.nix {
        inherit
          inputs
          pkgs
          system
          ortVersion
          ;
        src = inputs.self;
      };
    };
}
