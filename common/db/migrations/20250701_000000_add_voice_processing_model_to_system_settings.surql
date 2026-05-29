DEFINE FIELD IF NOT EXISTS voice_processing_model ON system_settings TYPE string;

UPDATE system_settings:current SET
    voice_processing_model = "whisper-1"
WHERE voice_processing_model == NONE; 