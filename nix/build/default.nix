{
  inputs,
  pkgs,
  system,
  src,
  ortVersion,
}:
let
  versions = import ../versions.nix;

  surrealdbPkg = import ../packages/surrealdb { inherit pkgs system; };

  cargo = import ./cargo.nix {
    inherit
      inputs
      pkgs
      system
      src
      ortVersion
      versions
      ;
  };

  windows = import ./windows.nix {
    inherit
      pkgs
      system
      ortVersion
      versions
      ;
    inherit (cargo)
      craneLib
      rustToolchain
      commonArgs
      minneVersion
      releaseCommonArgs
      ;
  };

  vmSmokeTest =
    if system == "x86_64-linux" then
      pkgs.callPackage ../tests/vm-smoke.nix {
        inherit (cargo) minne-pkg;
        inherit surrealdbPkg;
        minneNixosModule = import ../nixos/minne.nix { };
      }
    else
      null;

  moduleEvalTest =
    if pkgs.stdenv.isLinux then
      pkgs.callPackage ../tests/module-eval.nix {
        inherit (cargo) minne-pkg;
        inherit surrealdbPkg;
        minneNixosModule = import ../nixos/minne.nix { };
        inherit (inputs.nixpkgs.lib) nixosSystem;
      }
    else
      null;
in
cargo
// windows
// {
  inherit
    src
    ortVersion
    surrealdbPkg
    vmSmokeTest
    moduleEvalTest
    ;
}
