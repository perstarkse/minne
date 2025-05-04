# Defines the schema for the 'ingestion_task' table (used by IngestionTask).

DEFINE TABLE IF NOT EXISTS job SCHEMALESS;

# Standard fields
DEFINE FIELD IF NOT EXISTS created_at ON job TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON job TYPE string;

# Custom fields from the IngestionTask struct
# IngestionPayload is complex, store as object
DEFINE FIELD IF NOT EXISTS content ON job TYPE object;
# IngestionTaskStatus can hold data (InProgress), store as object
DEFINE FIELD IF NOT EXISTS status ON job TYPE object;
DEFINE FIELD IF NOT EXISTS user_id ON job TYPE string;

# Indexes explicitly defined in build_indexes and useful for get_unfinished_tasks
DEFINE INDEX IF NOT EXISTS idx_job_status ON job FIELDS status;
DEFINE INDEX IF NOT EXISTS idx_job_user ON job FIELDS user_id;
DEFINE INDEX IF NOT EXISTS idx_job_created ON job FIELDS created_at;
