{ versions, ... }:
let
  inherit (versions) ortVersion;
in
{
  perSystem =
    {
      pkgs,
      lib,
      minneCtx,
      ...
    }:
    let
      inherit (minneCtx)
        craneLib
        rustToolchain
        commonArgs
        minne-pkg
        vmSmokeTest
        moduleEvalTest
        src
        ;
    in
    {
      checks = {
        ortVersion = pkgs.runCommand "ort-version-check" { } ''
          if [ "${pkgs.onnxruntime.version}" != "${ortVersion}" ]; then
            echo "pkgs.onnxruntime.version is ${pkgs.onnxruntime.version}, but flake pins ${ortVersion}" >&2
            echo "Update nix/versions.nix or wait for nixpkgs to catch up." >&2
            exit 1
          fi
          touch $out
        '';

        rustToolchain =
          pkgs.runCommand "rust-toolchain-check"
            {
              nativeBuildInputs = [ rustToolchain.toolchain ];
            }
            ''
              expected="${rustToolchain.rustVersion}"
              actual="$(${rustToolchain.toolchain}/bin/rustc --version | awk '{print $2}')"
              if [ "$actual" != "$expected" ]; then
                echo "rustc version mismatch: expected $expected, got $actual" >&2
                echo "Update rustVersion in nix/versions.nix and rebuild the fenix toolchain." >&2
                exit 1
              fi
              touch $out
            '';

        minne-clippy = craneLib.cargoClippy (
          commonArgs
          // {
            cargoArtifacts = minne-pkg;
            pname = "minne";
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          }
        );

        minne-test = craneLib.cargoTest (
          commonArgs
          // {
            cargoArtifacts = minne-pkg;
            pname = "minne";
            buildInputs = commonArgs.buildInputs ++ [ pkgs.cacert ];
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-certificates.crt";
            cargoTestExtraArgs = "--workspace";
          }
        );

        minne-deny = craneLib.cargoDeny (
          commonArgs
          // {
            cargoArtifacts = minne-pkg;
            pname = "minne";
            cargoDenyChecks = "bans licenses sources";
          }
        );

        deadnix = pkgs.runCommand "deadnix-check" { nativeBuildInputs = [ pkgs.deadnix ]; } ''
          deadnix --fail ${src}/nix ${src}/flake.nix
          touch $out
        '';

        statix = pkgs.runCommand "statix-check" { nativeBuildInputs = [ pkgs.statix ]; } ''
          statix check ${src}
          touch $out
        '';

        actionlint =
          pkgs.runCommand "actionlint-check"
            {
              nativeBuildInputs = [
                pkgs.actionlint
                pkgs.shellcheck
              ];
            }
            ''
              cd ${src}
              shopt -s nullglob
              files=(.github/workflows/*.yml .github/workflows/*.yaml)
              actionlint "''${files[@]}"
              touch $out
            '';

        typos = pkgs.runCommand "typos-check" { nativeBuildInputs = [ pkgs.typos ]; } ''
          cd ${src}
          typos
          touch $out
        '';
      }
      // lib.optionalAttrs (vmSmokeTest != null) {
        minne-vm-smoke = vmSmokeTest;
      }
      // lib.optionalAttrs (moduleEvalTest != null) {
        minne-module-eval = moduleEvalTest;
      };
    };
}
