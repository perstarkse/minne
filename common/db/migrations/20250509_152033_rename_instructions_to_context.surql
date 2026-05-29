DEFINE FIELD IF NOT EXISTS context ON text_content TYPE option<string>;
DEFINE FIELD OVERWRITE instructions ON text_content TYPE option<string>;

UPDATE text_content SET context = instructions WHERE instructions IS NOT NONE;

UPDATE text_content UNSET instructions;

REMOVE FIELD instructions ON TABLE text_content;
