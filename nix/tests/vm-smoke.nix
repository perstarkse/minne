{
  pkgs,
  minne-pkg,
  surrealdbPkg,
  minneNixosModule,
}:
pkgs.testers.nixosTest {
  name = "minne-smoke";

  nodes.machine =
    { ... }:
    {
      imports = [ minneNixosModule ];

      virtualisation.memorySize = 4096;

      services.minne = {
        enable = true;
        package = minne-pkg;
        surrealdb = {
          enable = true;
          package = surrealdbPkg;
          username = "root_user";
          password = "root_password";
          namespace = "test";
          database = "test";
        };
        environment = {
          OPENAI_API_KEY = "test-key";
          STORAGE = "local";
          EMBEDDING_BACKEND = "hashed";
          INDEX_REBUILD_INTERVAL_SECS = "0";
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
