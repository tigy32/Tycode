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
//! ### context.rs
//! Provides types and helpers to build AI message context:
//! - Provides directory listing and contents of all tracked files
//!
//! ### manager.rs
//! Ties everything together and offers high-level APIs:
//! - Coordinates access.rs, security.rs for safe file modifications
//!
//! ## Multiple workspaces and Obfuscation
//! Tycode supports multiple workspaces (typically multiple git root projects
//! open in the same vscode window). This introduces complexity - each
//! workspace likely has its own .gitignore and we need a way to address files
//! between workspaces. To keep things simple for the AI agents, we present a
//! file system as if each workspace is its own root. For example, two
//! workspaces 'asdf' and 'zxcv' would be presented as `/asdf/src/file.rs` and
//! `zxcv/src/mod.rs` (for example). resolver.rs is responsible for mapping
//! from these fake root directories to real directories. This also hides real
//! directories (for example user names) from AI providers.

pub mod access;
pub mod config;
pub mod find;
pub mod manager;
pub mod modify;
pub mod read_only;
pub mod resolver;
pub mod search;
