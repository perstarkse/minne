# Flake checks: version gates, clippy, tests, formatting, VM smoke.
{
  ortVersion,
  ...
}:
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
        minneVersion
        vmSmokeTest
        src
        ;
    in
    {
      checks = {
        ortVersion = pkgs.runCommand "ort-version-check" { } ''
          if [ "${pkgs.onnxruntime.version}" != "${ortVersion}" ]; then
            echo "pkgs.onnxruntime.version is ${pkgs.onnxruntime.version}, but flake pins ${ortVersion}" >&2
            echo "Update ortVersion in flake.nix or wait for nixpkgs to catch up." >&2
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
                echo "Update rust-toolchain.toml and rebuild the fenix toolchain." >&2
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
            cargoTestExtraArgs = "--lib --bins";
          }
        );

        minne-fmt = craneLib.cargoFmt {
          pname = "minne-fmt";
          version = minneVersion;
          src = craneLib.cleanCargoSource src;
        };
      }
      // lib.optionalAttrs (vmSmokeTest != null) {
        minne-vm-smoke = vmSmokeTest;
      };
    };
}
