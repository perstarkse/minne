-- State machine migration for ingestion_task records

DEFINE FIELD IF NOT EXISTS state ON TABLE ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS attempts ON TABLE ingestion_task TYPE option<number>;
DEFINE FIELD IF NOT EXISTS max_attempts ON TABLE ingestion_task TYPE option<number>;
DEFINE FIELD IF NOT EXISTS scheduled_at ON TABLE ingestion_task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS locked_at ON TABLE ingestion_task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS lease_duration_secs ON TABLE ingestion_task TYPE option<number>;
DEFINE FIELD IF NOT EXISTS worker_id ON TABLE ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error_code ON TABLE ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error_message ON TABLE ingestion_task TYPE option<string>;
DEFINE FIELD IF NOT EXISTS last_error_at ON TABLE ingestion_task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS priority ON TABLE ingestion_task TYPE option<number>;

REMOVE FIELD status ON TABLE ingestion_task;
DEFINE FIELD status ON TABLE ingestion_task TYPE option<object>;

DEFINE INDEX IF NOT EXISTS idx_ingestion_task_state_sched ON TABLE ingestion_task FIELDS state, scheduled_at;

LET $needs_migration = (SELECT count() AS count FROM type::table('ingestion_task') WHERE state = NONE)[0].count;

IF $needs_migration > 0 THEN {
    -- Created -> Pending
    UPDATE type::table('ingestion_task')
    SET
        state = "Pending",
        attempts = 0,
        max_attempts = 3,
        scheduled_at = IF created_at != NONE THEN created_at ELSE time::now() END,
        locked_at = NONE,
        lease_duration_secs = 300,
        worker_id = NONE,
        error_code = NONE,
        error_message = NONE,
        last_error_at = NONE,
        priority = 0
    WHERE state = NONE
      AND status != NONE
      AND status.name = "Created";

    -- InProgress -> Processing
    UPDATE type::table('ingestion_task')
    SET
        state = "Processing",
        attempts = IF status.attempts != NONE THEN status.attempts ELSE 1 END,
        max_attempts = 3,
        scheduled_at = IF status.last_attempt != NONE THEN status.last_attempt ELSE time::now() END,
        locked_at = IF status.last_attempt != NONE THEN status.last_attempt ELSE time::now() END,
        lease_duration_secs = 300,
        worker_id = NONE,
        error_code = NONE,
        error_message = NONE,
        last_error_at = NONE,
        priority = 0
    WHERE state = NONE
      AND status != NONE
      AND status.name = "InProgress";

    -- Completed -> Succeeded
    UPDATE type::table('ingestion_task')
    SET
        state = "Succeeded",
        attempts = 1,
        max_attempts = 3,
        scheduled_at = IF updated_at != NONE THEN updated_at ELSE time::now() END,
        locked_at = NONE,
        lease_duration_secs = 300,
        worker_id = NONE,
        error_code = NONE,
        error_message = NONE,
        last_error_at = NONE,
        priority = 0
    WHERE state = NONE
      AND status != NONE
      AND status.name = "Completed";

    -- Error -> DeadLetter (terminal failure)
    UPDATE type::table('ingestion_task')
    SET
        state = "DeadLetter",
        attempts = 3,
        max_attempts = 3,
        scheduled_at = IF updated_at != NONE THEN updated_at ELSE time::now() END,
        locked_at = NONE,
        lease_duration_secs = 300,
        worker_id = NONE,
        error_code = NONE,
        error_message = status.message,
        last_error_at = IF updated_at != NONE THEN updated_at ELSE time::now() END,
        priority = 0
    WHERE state = NONE
      AND status != NONE
      AND status.name = "Error";

    -- Cancelled -> Cancelled
    UPDATE type::table('ingestion_task')
    SET
        state = "Cancelled",
        attempts = 0,
        max_attempts = 3,
        scheduled_at = IF updated_at != NONE THEN updated_at ELSE time::now() END,
        locked_at = NONE,
        lease_duration_secs = 300,
        worker_id = NONE,
        error_code = NONE,
        error_message = NONE,
        last_error_at = NONE,
        priority = 0
    WHERE state = NONE
      AND status != NONE
      AND status.name = "Cancelled";

    -- Fallback for any remaining records missing state
    UPDATE type::table('ingestion_task')
    SET
        state = "Pending",
        attempts = 0,
        max_attempts = 3,
        scheduled_at = IF updated_at != NONE THEN updated_at ELSE time::now() END,
        locked_at = NONE,
        lease_duration_secs = 300,
        worker_id = NONE,
        error_code = NONE,
        error_message = NONE,
        last_error_at = NONE,
        priority = 0
    WHERE state = NONE;
} END;

-- Ensure defaults for newly added fields
UPDATE type::table('ingestion_task')
SET max_attempts = 3
WHERE max_attempts = NONE;

UPDATE type::table('ingestion_task')
SET lease_duration_secs = 300
WHERE lease_duration_secs = NONE;

UPDATE type::table('ingestion_task')
SET attempts = 0
WHERE attempts = NONE;

UPDATE type::table('ingestion_task')
SET priority = 0
WHERE priority = NONE;

UPDATE type::table('ingestion_task')
SET scheduled_at = IF updated_at != NONE THEN updated_at ELSE time::now() END
WHERE scheduled_at = NONE;

UPDATE type::table('ingestion_task')
SET locked_at = NONE
WHERE locked_at = NONE;

UPDATE type::table('ingestion_task')
SET worker_id = NONE
WHERE worker_id != NONE AND worker_id = "";

UPDATE type::table('ingestion_task')
SET error_code = NONE
WHERE error_code = NONE;

UPDATE type::table('ingestion_task')
SET error_message = NONE
WHERE error_message = NONE;

UPDATE type::table('ingestion_task')
SET last_error_at = NONE
WHERE last_error_at = NONE;

UPDATE type::table('ingestion_task')
SET status = NONE
WHERE status != NONE;
