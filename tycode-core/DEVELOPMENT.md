# tycode-core Development Guide

## Core Philosophy

### tycode-core as a Library

tycode-core is designed as a modular library where all core modules are optional:

- `ChatActorBuilder::new()` creates a bare AI chatbot with no instructions
- `ChatActorBuilder::tycode()` installs all first-party Tycode modules
- Developers can opt out of all modules and install their own for custom applications

### Module Parity

First-party and third-party modules are treated identically:

- No special privileges or access for built-in modules
- All modules use the same `Module` trait and registration APIs
- The same patterns used internally are available externally

## Module Architecture

### Self-Containment Principle

- ActorState should NOT know about specific modules
- Small single-file modules can live in `src/modules/module_name.rs` (e.g., `src/modules/execution.rs`, `src/modules/task_list.rs`)
- Multi-file modules should have their own top-level folder under `src/`:
  - Each multi-file module gets its own folder (e.g., `src/analyzer/`, `src/memory/`)
  - The folder contains all implementation files AND the `Module` impl in `mod.rs`
  - Do NOT split module definition into `src/modules/` separate from implementation for multi-file modules
- All module state, tools, prompts, and context components belong together

### Module Trait

```rust
pub trait Module: Send + Sync {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>>;
    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>>;
    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>>;
}
```

Modules bundle related:

- **Prompt components**: Instructions for the AI
- **Context components**: Runtime information included in messages
- **Tools**: Actions the AI can take

## Testing Philosophy

### Zero Unit Tests

We do not write unit tests for module internals. All tests are "simulation tests" in `tests/*`.

### Simulation Tests

- Test through the actor using the public API
- Zero coupling to module implementation
- A module can be completely rewritten and all tests should pass (if functionality is identical)
- Each module has its own test file: `tests/task_list.rs`, `tests/memory.rs`, etc.

### Module Tests Location

Tests for modules in `src/modules/` should be placed in `tests/modules/`:

- File names must match: `src/modules/execution.rs` → `tests/modules/execution.rs`
- The `tests/modules.rs` file declares all module tests with path attributes
- To add a new module test, add `#[path = "modules/your_module.rs"] mod your_module;` to `tests/modules.rs`


See `tests/TESTING.MD` for detailed testing patterns and examples.

### Test Coverage Improvement

Our test coverage is improving over time. The rule for every bug fix:

1. Write a failing simulation test that reproduces the bug
2. Fix the bug
3. Verify the test passes with the fix

This ensures we never regress on fixed bugs and builds coverage organically.

## Creating a New Module

1. For single-file modules: create `src/modules/my_module.rs`
   For multi-file modules: create folder `src/my_module/` with `mod.rs`
2. Add implementation files (tools, context components, etc.) in that folder
3. Implement the `Module` trait in `mod.rs` or a dedicated `module.rs` file within the folder
4. Register in `ChatActorBuilder::tycode()` or via `with_module()`
5. Create `tests/modules/my_module.rs` with simulation tests
6. Document in module-level doc comments

Example structure:
```
src/analyzer/
  mod.rs           # exports + AnalyzerModule impl
  search_types.rs  # tool implementation
  get_type_docs.rs # tool implementation
  rust_analyzer.rs # shared implementation
```
