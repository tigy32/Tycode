//! Module-level simulation tests.
//!
//! Each module in `src/modules/` should have a corresponding test file here
//! with the same name (e.g., `execution.rs` tests `modules/execution.rs`).

#[path = "modules/execution.rs"]
mod execution;

#[path = "modules/task_list.rs"]
mod task_list;

#[path = "modules/memory.rs"]
mod memory;

#[path = "modules/file.rs"]
mod file;

#[path = "modules/image.rs"]
mod image;

#[path = "modules/spawn.rs"]
mod spawn;

#[path = "modules/review.rs"]
mod review;

#[path = "modules/context_management.rs"]
mod context_management;
