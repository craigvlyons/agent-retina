use retina_types::{ConsolidationConfig, ConsolidationReport, Result};

use crate::controller::InspectController;
use crate::runtime::init_runtime;

pub fn run_cleanup(
    keep_events: usize,
    stale_knowledge_days: u64,
    optimize: bool,
) -> Result<ConsolidationReport> {
    init_runtime()?;
    let inspector = InspectController::new()?;
    inspector.cleanup_memory(ConsolidationConfig {
        max_recent_states: keep_events,
        stale_knowledge_days: Some(stale_knowledge_days),
        optimize_after_cleanup: optimize,
        ..ConsolidationConfig::default()
    })
}
