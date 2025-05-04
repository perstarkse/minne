# Ensure 'analytics:current' record exists
CREATE analytics:current CONTENT {
    page_loads: 0,
    visitors: 0,
}; 

# Ensure 'system_settings:current' record exists
CREATE system_settings:current CONTENT {
    registrations_enabled: true,
    require_email_verification: false,
    query_model: "gpt-4o-mini",
    processing_model: "gpt-4o-mini",
    query_system_prompt: "<Your default query prompt here>",      
    ingestion_system_prompt: "<Your default ingress prompt here>" 
};
