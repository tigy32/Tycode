//! The file module provides a structured, secure interface to file system operations.
//!
//! ## Architecture
//!
//! ### access.rs
//! Provides low-level file system access with safety guards:
//! - Ignores critical safety risks (e.g., .git directories, dot directories)
//! - Respects .gitignore patterns via the `ignore` crate's WalkBuilder (files in ignored locations appear non-existent)
//! - Offers core APIs: read_file, write_file, delete_file, list_directory
//! - All I/O goes through this layer; nothing uses std::fs directly
//! - File discovery uses ignore::WalkBuilder to traverse directories while respecting ignore patterns and size limits
//!
//! ### manager.rs
//! Ties everything together and offers high-level APIs:
//! - Coordinates access.rs, security.rs for safe file modifications
//!
//! ## Multiple workspaces
//! Tycode supports multiple workspace roots (typically multiple git root
//! projects open in the same VS Code window). File tools show and accept real
//! absolute paths, and access.rs enforces that file operations stay inside one
//! of the configured roots.

pub mod access;
pub mod config;
pub mod find;
pub mod manager;
pub mod modify;
pub mod read_only;
pub mod workspace;
