-- Track scheduled runtime index rebuild state on the system_settings singleton.

DEFINE FIELD IF NOT EXISTS last_index_rebuild_at ON system_settings TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS index_rebuild_lease_owner ON system_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS index_rebuild_lease_expires_at ON system_settings TYPE option<datetime>;
