// Runtime owns task supervision mechanics only. Keep strategy, routing, and
// model guidance out of this crate.
mod command;
mod model;
mod output;
mod registry;
mod supervisor;
mod timeline;

pub use command::*;
pub use model::*;
pub use output::{TaskOutputDelta, compact_summary, outcome_summary, read_output_delta};
pub use registry::*;
pub use supervisor::*;
