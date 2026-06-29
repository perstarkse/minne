_: {
  perSystem =
    {
      config,
      minneCtx,
      ...
    }:
    let
      inherit (minneCtx) rustToolchain;
    in
    {
      treefmt = {
        projectRootFile = "flake.nix";

        programs = {
          nixfmt.enable = true;
          rustfmt = {
            enable = true;
            edition = "2024";
            package = rustToolchain.toolchain;
          };
        };

        settings.global.excludes = [
          "**/.direnv/**"
          "**/.data/**"
          "**/database/**"
          "**/evaluations/cache/**"
          "**/evaluations/reports/**"
          "**/html-router/node_modules/**"
          "**/result/**"
          "**/target/**"
        ];
      };

      apps.fmt = {
        type = "app";
        program = "${config.treefmt.build.wrapper}/bin/treefmt";
        meta.description = "Format the repository (rustfmt, nixfmt)";
      };
    };
}
