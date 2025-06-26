# Defines the schema for the 'ingestion_task' table (used by IngestionTask).

DEFINE TABLE IF NOT EXISTS ingestion_task SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON ingestion_task TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON ingestion_task TYPE string;

# Custom fields from the IngestionTask struct
# IngestionPayload is complex, store as object
DEFINE FIELD IF NOT EXISTS content ON ingestion_task TYPE object;
# IngestionTaskStatus can hold data (InProgress), store as object
DEFINE FIELD IF NOT EXISTS status ON ingestion_task TYPE object;
DEFINE FIELD IF NOT EXISTS user_id ON ingestion_task TYPE string;

# Indexes explicitly defined in build_indexes and useful for get_unfinished_tasks
DEFINE INDEX IF NOT EXISTS idx_ingestion_task_status ON ingestion_task FIELDS status;
DEFINE INDEX IF NOT EXISTS idx_ingestion_task_user ON ingestion_task FIELDS user_id;
DEFINE INDEX IF NOT EXISTS idx_ingestion_task_created ON ingestion_task FIELDS created_at;
