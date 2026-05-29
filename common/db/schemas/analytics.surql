# Defines the schema for the 'analytics' table.

DEFINE TABLE IF NOT EXISTS analytics SCHEMALESS;

# Custom fields from the Analytics struct
DEFINE FIELD IF NOT EXISTS page_loads ON analytics TYPE number;
DEFINE FIELD IF NOT EXISTS visitors ON analytics TYPE number;
