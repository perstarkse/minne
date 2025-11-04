use crate::slices::SliceConfig as CoreSliceConfig;

pub use crate::slices::*;

use crate::args::Config;

impl<'a> From<&'a Config> for CoreSliceConfig<'a> {
    fn from(config: &'a Config) -> Self {
        slice_config_with_limit(config, None)
    }
}

pub fn slice_config_with_limit<'a>(
    config: &'a Config,
    limit_override: Option<usize>,
) -> CoreSliceConfig<'a> {
    CoreSliceConfig {
        cache_dir: config.cache_dir.as_path(),
        force_convert: config.force_convert,
        explicit_slice: config.slice.as_deref(),
        limit: limit_override.or(config.limit),
        corpus_limit: config.corpus_limit,
        slice_seed: config.slice_seed,
        llm_mode: config.llm_mode,
        negative_multiplier: config.negative_multiplier,
    }
}
