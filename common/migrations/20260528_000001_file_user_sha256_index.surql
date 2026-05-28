-- Per-user deduplication: same SHA256 may exist for different users.
REMOVE INDEX IF EXISTS file_sha256_idx ON file;
DEFINE INDEX IF NOT EXISTS file_user_sha256_idx ON file FIELDS user_id, sha256 UNIQUE;
