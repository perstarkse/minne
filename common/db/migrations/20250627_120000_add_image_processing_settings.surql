DEFINE FIELD IF NOT EXISTS image_processing_model ON system_settings TYPE string;
DEFINE FIELD IF NOT EXISTS image_processing_prompt ON system_settings TYPE string;

UPDATE system_settings:current SET
	image_processing_model =  "gpt-4o-mini",
    image_processing_prompt = "Analyze this image and respond based on its primary content:\n - If the image is mainly text (document, screenshot, sign), transcribe the text verbatim.\n - If the image is mainly visual (photograph, art, landscape), provide a concise description of the scene.\n - For hybrid images (diagrams, ads), briefly describe the visual, then transcribe the text under a Text: heading.\n\n Respond directly with the analysis."
WHERE image_processing_model == NONE && image_processing_prompt == NONE;
