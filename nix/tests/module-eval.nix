{
  lib,
  pkgs,
  minne-pkg,
  surrealdbPkg,
  minneNixosModule,
  nixosSystem,
}:
let
  evalModule =
    module:
    (nixosSystem {
      inherit (pkgs.stdenv.hostPlatform) system;
      modules = [
        minneNixosModule
        {
          nixpkgs.pkgs = pkgs;
          boot.loader.grub.enable = false;
          fileSystems."/".device = "/dev/sda1";
          system.stateVersion = "25.11";
        }
        module
      ];
    }).config;

  combined = evalModule {
    services.minne = {
      enable = true;
      package = minne-pkg;
    };
  };

  split = evalModule {
    services.minne = {
      enable = true;
      package = minne-pkg;
      mode = "split";
    };
  };

  external = evalModule {
    services.minne = {
      enable = true;
      package = minne-pkg;
      surrealdb = {
        enable = false;
        host = "db.internal";
        port = 9999;
        after = [ "surrealdb.service" ];
        requires = [ "surrealdb.service" ];
      };
    };
  };

  bundled = evalModule {
    services.minne = {
      enable = true;
      package = minne-pkg;
      surrealdb = {
        enable = true;
        package = surrealdbPkg;
      };
    };
  };

  hasUnit = cfg: name: cfg.systemd.services ? ${name};

  assertions = [
    {
      assertion = hasUnit combined "minne" && !(hasUnit combined "minne-server");
      message = "combined mode must define only the `minne` unit.";
    }
    {
      assertion = combined.systemd.services.minne.serviceConfig.ExecStart == "${minne-pkg}/bin/main";
      message = "combined mode must run the `main` binary.";
    }
    {
      assertion =
        hasUnit split "minne-server" && hasUnit split "minne-worker" && !(hasUnit split "minne");
      message = "split mode must define `minne-server` and `minne-worker`, not `minne`.";
    }
    {
      assertion =
        split.systemd.services.minne-server.serviceConfig.ExecStart == "${minne-pkg}/bin/server";
      message = "split mode `minne-server` must run the `server` binary.";
    }
    {
      assertion =
        split.systemd.services.minne-worker.serviceConfig.ExecStart == "${minne-pkg}/bin/worker";
      message = "split mode `minne-worker` must run the `worker` binary.";
    }
    {
      assertion = !(hasUnit external "surrealdb");
      message = "external SurrealDB must not define a bundled `surrealdb` unit.";
    }
    {
      assertion =
        external.systemd.services.minne.environment.SURREALDB_ADDRESS == "ws://db.internal:9999";
      message = "external SurrealDB host/port must be threaded into SURREALDB_ADDRESS.";
    }
    {
      assertion =
        lib.elem "surrealdb.service" external.systemd.services.minne.after
        && lib.elem "surrealdb.service" external.systemd.services.minne.requires;
      message = "external SurrealDB after/requires must order Minne after the DB unit.";
    }
    {
      assertion = hasUnit bundled "surrealdb";
      message = "bundled SurrealDB must define the `surrealdb` unit.";
    }
    {
      assertion = lib.hasPrefix "${surrealdbPkg}/bin/surreal" bundled.systemd.services.surrealdb.serviceConfig.ExecStart;
      message = "bundled SurrealDB unit must use the overridden `surrealdb.package`.";
    }
  ];

  failures = lib.filter (a: !a.assertion) assertions;
in
pkgs.runCommand "minne-module-eval-checks" { } (
  if failures == [ ] then
    "touch $out"
  else
    ''
      echo "services.minne module shape checks failed:" >&2
      ${lib.concatMapStringsSep "\n" (a: "echo '  - ${a.message}' >&2") failures}
      exit 1
    ''
)
