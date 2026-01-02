//! Module-level simulation tests.
//!
//! Each module in `src/modules/` should have a corresponding test file here
//! with the same name (e.g., `execution.rs` tests `modules/execution.rs`).

#[path = "modules/execution.rs"]
mod execution;

#[path = "modules/task_list.rs"]
mod task_list;
