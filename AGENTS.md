# AGENTS.md

Guidance for AI coding agents working in the Tycode repository. These
conventions apply to every agent that touches this codebase.

## Repository hygiene

- Start by checking `git status --short` and identifying any existing user
  changes. Do not overwrite or reformat unrelated dirty files.
- Keep changes scoped to the user request. Avoid drive-by refactors unless the
  user explicitly asks for them or they are required to fix a failing check.
- Prefer editing existing files over adding new abstractions.
- Match nearby code style and architecture. Tycode is built around
  `tycode-core` modules, the `ChatActor` protocol, provider implementations,
  and the TypeScript/VS Code subprocess bridge.
- Do not push, tag, open PRs, force-push, or otherwise affect remotes without
  explicit user approval.

## Commit workflow

1. Inspect the diff before committing:
   - `git status --short`
   - `git diff --stat`
   - `git diff`
2. Run the relevant validation commands from the test workflow below.
3. Stage only the intended files.
4. Commit locally.
5. Confirm the result:
   - `git status --short`
   - `git log -1 --oneline`

If the user asks you to sync first, use `git fetch` followed by
`git pull --rebase --autostash`. Stop and report conflicts instead of
silently discarding work.

## Commit message rules

- Use the imperative mood: `Add Fable model support`, not `Added...`.
- Capitalize the subject line.
- Keep the subject line at or under 50 characters.
- Do not end the subject with a period.
- If a body is needed, separate it from the subject with a blank line and wrap
  it at 72 columns.
- Explain what and why in the body; the diff explains how.
- Do not add AI attribution or tool trailers such as `Co-authored-by`,
  `Generated with`, or similar footers.

## User-facing messages

- Be concise, friendly, and concrete.
- State exactly what changed and which commit was created.
- List validation commands that were run and whether they passed.
- If a check fails, include the command, the failure summary, and whether it
  appears related to the current change or pre-existing.
- Do not claim a push, release, or remote update happened unless it actually
  did and the user approved it.

## Test workflow

Run the smallest set that proves the change, then broaden before committing
code that affects shared behavior.

### Rust core, CLI, and subprocess

- Format: `cargo fmt --all`
- Compile: `cargo check --workspace`
- Full CI-equivalent Rust tests: `cargo nextest run --workspace --profile ci`
- Targeted iteration: `cargo test -p tycode-core <test-filter> -- --nocapture`

Use targeted tests while iterating, but prefer the CI-equivalent nextest command
before committing broad Rust changes.

### TypeScript client

From `tycode-client-typescript/`:

- Build/types: `npm run build`
- Integration tests: `npm test -- --runInBand`

Run these when touching `tycode-client-typescript`, `tycode-subprocess`, the
JSON protocol, generated/copied client types, or provider/settings behavior
that the client exercises.

### VS Code extension

From `tycode-vscode/`:

- Compile/package build path: `npm run compile`
- Extension tests, when UI or extension behavior changes: `npm test`

`npm run compile` also rebuilds the TypeScript client and collects the local
`tycode-subprocess` binary, so expect generated ignored artifacts under
`tycode-vscode/out`, `tycode-vscode/lib`, `tycode-vscode/bin`, and
`tycode-vscode/src/build-info.ts`.

### Docs-only changes

For documentation-only changes, `git diff --check` is usually sufficient.
Run broader tests if the docs change scripts, commands, release instructions,
or any executable examples.

## Handling failing checks

- Fix failures caused by your change before committing.
- If a failure is pre-existing, preserve the evidence: command, failure text,
  and why it appears unrelated.
- Do not weaken or delete tests to make them pass unless the user agrees the
  test is wrong.
- Prefer fixing nearby collateral issues when they block the requested commit;
  mention them in the commit body if they are not obvious from the subject.

## Release workflow

Only perform a release after explicit user approval of the exact target
version. Before tagging or pushing anything:

1. Confirm `git status --short` is clean and `git branch --show-current` is
   `main`.
2. Confirm the release commit contains the intended version updates.
3. Run the relevant full validation workflow.
4. Confirm the tag does not already exist locally or on `origin`.
5. Create the annotated tag locally.
6. Push `main` and the tag only after the user explicitly approves the push.
