{
  surreal = {
    host = "127.0.0.1";
    port = 8000;
    user = "root_user";
    pass = "root_password";
    namespace = "minne_ns";
    database = "minne_db";
  };

  minio = {
    endpoint = "http://127.0.0.1:19000";
    accessKey = "minioadmin";
    secretKey = "minioadmin";
    bucket = "minne-tests";
    region = "us-east-1";
  };

  app = {
    httpPort = 3009;
    dataDir = "./data";
    # Replace in `.env.local` for real LLM use.
    openaiApiKey = "local-dev-placeholder";
    embeddingBackend = "fastembed";
    pdfIngestMode = "classic";
    storage = "local";
    rustLog = "info";
  };
}
