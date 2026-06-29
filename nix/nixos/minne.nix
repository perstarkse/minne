{
  self ? null,
}:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.minne;

  surrealAddress = "ws://${cfg.surrealdb.host}:${toString cfg.surrealdb.port}";

  surrealUnits =
    lib.optional cfg.surrealdb.enable "surrealdb.service"
    ++ cfg.surrealdb.after
    ++ cfg.surrealdb.requires;

  minneAfter = [ "network.target" ] ++ surrealUnits;
  minneRequires = surrealUnits;

  surrealAuthArgs =
    if cfg.surrealdb.username != null then
      "--user ${cfg.surrealdb.username} --pass ${cfg.surrealdb.password}"
    else
      "";

  baseEnvironment = {
    SURREALDB_ADDRESS = surrealAddress;
    HTTP_PORT = toString cfg.port;
    RUST_LOG = cfg.logLevel;
    DATA_DIR = cfg.dataDir;
    SURREALDB_NAMESPACE = cfg.surrealdb.namespace;
    SURREALDB_DATABASE = cfg.surrealdb.database;
  }
  // lib.optionalAttrs (cfg.surrealdb.username != null) {
    SURREALDB_USERNAME = cfg.surrealdb.username;
    SURREALDB_PASSWORD = cfg.surrealdb.password;
  }
  // cfg.environment;

  mkService =
    { description, bin }:
    {
      inherit description;
      wantedBy = [ "multi-user.target" ];
      after = minneAfter;
      requires = minneRequires;
      environment = baseEnvironment;
      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.dataDir;
        ExecStart = "${cfg.package}/bin/${bin}";
        Restart = "always";
        RestartSec = "10";
        EnvironmentFile = lib.optional (cfg.environmentFile != null) cfg.environmentFile;
      };
    };
in
{
  options.services.minne = {
    enable = lib.mkEnableOption "the Minne knowledge management server";

    package = lib.mkOption {
      type = lib.types.package;
      default =
        if self != null then
          self.packages.${pkgs.stdenv.hostPlatform.system}.default
        else
          throw "services.minne.package must be set when the Minne flake is not available";
      defaultText = lib.literalExpression "minne.packages.\${system}.default";
      description = "Minne package providing the main/server/worker binaries.";
    };

    mode = lib.mkOption {
      type = lib.types.enum [
        "combined"
        "split"
      ];
      default = "combined";
      description = ''
        "combined" runs a single `main` process (API, web UI, and worker).
        "split" runs the API-only `server` alongside a standalone `worker`.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "minne";
      description = "User account under which Minne runs.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "minne";
      description = "Group under which Minne runs.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "TCP port the Minne HTTP server listens on (HTTP_PORT).";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/minne";
      description = "State directory for Minne (DATA_DIR).";
    };

    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = "Value for RUST_LOG.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open Minne `port` in the firewall.";
    };

    environment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        EMBEDDING_BACKEND = "hashed";
      };
      description = "Extra environment variables passed to every Minne service.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = "/run/secrets/minne.env";
      description = ''
        Path to a systemd EnvironmentFile holding secrets such as
        `OPENAI_API_KEY`. SurrealDB credentials can be set via
        `services.minne.surrealdb` or included here.
      '';
    };

    surrealdb = {
      enable = lib.mkEnableOption "a bundled SurrealDB instance for Minne";

      package = lib.mkOption {
        type = lib.types.package;
        default = import ../packages/surrealdb { inherit pkgs; };
        description = "SurrealDB package used when `services.minne.surrealdb.enable` is true.";
      };

      host = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        description = "Host SurrealDB binds to and Minne connects to.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 8000;
        description = "Port SurrealDB listens on and Minne connects to.";
      };

      dataDir = lib.mkOption {
        type = lib.types.path;
        default = "/var/lib/surrealdb";
        description = "RocksDB data directory for bundled SurrealDB.";
      };

      user = lib.mkOption {
        type = lib.types.str;
        default = "surrealdb";
        description = "User account under which bundled SurrealDB runs.";
      };

      group = lib.mkOption {
        type = lib.types.str;
        default = "surrealdb";
        description = "Group under which bundled SurrealDB runs.";
      };

      namespace = lib.mkOption {
        type = lib.types.str;
        default = "minne";
        description = "SurrealDB namespace passed to Minne (SURREALDB_NAMESPACE).";
      };

      database = lib.mkOption {
        type = lib.types.str;
        default = "minne";
        description = "SurrealDB database passed to Minne (SURREALDB_DATABASE).";
      };

      username = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "root";
        description = ''
          Root username for bundled SurrealDB (`--user`). When set, `password`
          must also be set and Minne receives matching SURREALDB_USERNAME/PASSWORD.
        '';
      };

      password = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Root password for bundled SurrealDB (`--pass`).";
      };

      credentialsFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = "/run/secrets/surrealdb-credentials";
        description = ''
          Optional EnvironmentFile for bundled SurrealDB. Use when credentials
          should not appear in the Nix store. Expects SURREALDB_USER and
          SURREALDB_PASS (or SURREALDB_USERNAME / SURREALDB_PASSWORD).
        '';
      };

      openFirewall = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Open bundled SurrealDB `port` in the firewall.";
      };

      after = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "network-online.target" ];
        description = "Extra units ordered before Minne when using an external SurrealDB.";
      };

      requires = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "surrealdb.service" ];
        description = "Extra units Minne hard-depends on (external SurrealDB).";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          !cfg.surrealdb.enable || cfg.surrealdb.username == null || cfg.surrealdb.password != null;
        message = "services.minne.surrealdb.password must be set when username is set.";
      }
    ];

    systemd.services = lib.mkMerge [
      (lib.mkIf cfg.surrealdb.enable {
        surrealdb = {
          description = "SurrealDB for Minne";
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          serviceConfig = {
            Type = "simple";
            User = cfg.surrealdb.user;
            Group = cfg.surrealdb.group;
            WorkingDirectory = cfg.surrealdb.dataDir;
            ExecStart = "${cfg.surrealdb.package}/bin/surreal start --bind ${cfg.surrealdb.host}:${toString cfg.surrealdb.port} ${surrealAuthArgs} rocksdb:${cfg.surrealdb.dataDir}/data.db";
            Restart = "always";
            RestartSec = "10";
            EnvironmentFile = lib.optional (
              cfg.surrealdb.credentialsFile != null
            ) cfg.surrealdb.credentialsFile;
          };
        };
      })
      (lib.mkIf (cfg.mode == "combined") {
        minne = mkService {
          description = "Minne — API, web UI, and background worker";
          bin = "main";
        };
      })
      (lib.mkIf (cfg.mode == "split") {
        minne-server = mkService {
          description = "Minne — API-only server";
          bin = "server";
        };
        minne-worker = mkService {
          description = "Minne — standalone background worker";
          bin = "worker";
        };
      })
    ];

    users.users = lib.mkMerge [
      (lib.mkIf (cfg.user == "minne") {
        minne = {
          isSystemUser = true;
          inherit (cfg) group;
          home = cfg.dataDir;
          createHome = true;
        };
      })
      (lib.mkIf (cfg.surrealdb.enable && cfg.surrealdb.user == "surrealdb") {
        surrealdb = {
          isSystemUser = true;
          inherit (cfg.surrealdb) group;
          home = cfg.surrealdb.dataDir;
          createHome = true;
        };
      })
    ];

    users.groups = lib.mkMerge [
      (lib.mkIf (cfg.group == "minne") {
        minne = { };
      })
      (lib.mkIf (cfg.surrealdb.enable && cfg.surrealdb.group == "surrealdb") {
        surrealdb = { };
      })
    ];

    systemd.tmpfiles.rules = [
      "d ${cfg.dataDir} 0750 ${cfg.user} ${cfg.group} -"
    ]
    ++ lib.optional cfg.surrealdb.enable "d ${cfg.surrealdb.dataDir} 0750 ${cfg.surrealdb.user} ${cfg.surrealdb.group} -";

    networking.firewall.allowedTCPPorts =
      (lib.optional cfg.openFirewall cfg.port)
      ++ lib.optional (cfg.surrealdb.enable && cfg.surrealdb.openFirewall) cfg.surrealdb.port;
  };
}
