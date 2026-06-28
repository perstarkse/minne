# Crane packages, release archives, and flake apps.
{ ... }:
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
        minne-pkg
        minne-pkg-windows
        minne-release
        minne-release-windows
        dockerImage
        xwinCargoCache
        ;
    in
    {
      packages = {
        inherit minne-pkg dockerImage;
        default = minne-pkg;
      }
      // lib.optionalAttrs (minne-release != null) {
        minne-release = minne-release;
      }
      // lib.optionalAttrs (minne-release-windows != null) {
        inherit xwinCargoCache;
        minne-release-windows = minne-release-windows;
      };

      apps = {
        main = {
          type = "app";
          program = "${minne-pkg}/bin/main";
          meta.description = "Minne main server — API, web UI, and background worker";
        };
        worker = {
          type = "app";
          program = "${minne-pkg}/bin/worker";
          meta.description = "Minne standalone background worker (ingestion, indexing, maintenance)";
        };
        server = {
          type = "app";
          program = "${minne-pkg}/bin/server";
          meta.description = "Minne API-only server (no background worker)";
        };
        default = {
          type = "app";
          program = "${minne-pkg}/bin/main";
          meta.description = "Minne main server — API, web UI, and background worker";
        };
      };
    };
}
