# NixOS VM smoke test for the packaged Minne server.
{
  lib,
  pkgs,
  minne-pkg,
  surrealdb,
}:
pkgs.testers.nixosTest {
  name = "minne-smoke";

  nodes.machine = {
    virtualisation.memorySize = 4096;

    systemd.services.surrealdb = {
      description = "SurrealDB for Minne smoke test";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      preStart = "mkdir -p /var/lib/surrealdb";
      serviceConfig = {
        Type = "simple";
        ExecStart = "${surrealdb}/bin/surreal start --bind 127.0.0.1:8000 --user root_user --pass root_password rocksdb:/var/lib/surrealdb/db";
      };
    };

    systemd.services.minne = {
      description = "Minne server smoke test";
      wantedBy = [ "multi-user.target" ];
      after = [
        "surrealdb.service"
        "network.target"
      ];
      requires = [ "surrealdb.service" ];
      preStart = ''
        for i in $(seq 1 60); do
          if ${pkgs.netcat}/bin/nc -z 127.0.0.1 8000; then
            exit 0
          fi
          sleep 1
        done
        echo "surrealdb did not become ready on port 8000" >&2
        exit 1
      '';
      serviceConfig = {
        Type = "simple";
        ExecStart = "${minne-pkg}/bin/main";
        Environment = [
          "SURREALDB_ADDRESS=ws://127.0.0.1:8000"
          "SURREALDB_USERNAME=root_user"
          "SURREALDB_PASSWORD=root_password"
          "SURREALDB_NAMESPACE=test"
          "SURREALDB_DATABASE=test"
          "OPENAI_API_KEY=test-key"
          "HTTP_PORT=3000"
          "STORAGE=local"
          "DATA_DIR=/var/lib/minne"
          "EMBEDDING_BACKEND=hashed"
          "RUST_LOG=info"
          "INDEX_REBUILD_INTERVAL_SECS=0"
        ];
        StateDirectory = "minne";
      };
    };
  };

  testScript = ''
    machine.wait_for_unit("surrealdb.service")
    machine.wait_for_unit("minne.service")
    machine.wait_for_open_port(3000)
    machine.succeed("curl -sf http://127.0.0.1:3000/api/v1/live")
    machine.succeed("curl -sf http://127.0.0.1:3000/api/v1/ready")
  '';
}
