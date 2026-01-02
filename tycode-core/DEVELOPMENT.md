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
- Modules should be defined in a single location:
  - Small modules: single file (e.g., `tools/memory.rs`)
  - Large modules: folder with multiple files (e.g., `tools/tasks/`)
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

1. Choose location: `src/tools/my_module.rs` or `src/tools/my_module/mod.rs`
2. Implement the `Module` trait
3. Register in `ChatActorBuilder::tycode()` or via `with_module()`
4. Create `tests/my_module.rs` with simulation tests
5. Document in module-level doc comments
