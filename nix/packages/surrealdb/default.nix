{
  pkgs,
  system ? pkgs.stdenv.hostPlatform.system,
}:
if system == "x86_64-linux" then pkgs.callPackage ./binary.nix { } else pkgs.surrealdb
