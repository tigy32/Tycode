# Tycode - Claude Code Guidelines

## Git Remotes & Sync Flow

This repo has two remotes:
- **origin** (tigy32/Tycode) - the original upstream repo
- **fork** (k29/Tycode) - our fork

Our fork should stay synced with upstream. **Always prompt the user to sync before starting work.**

### Sync fork with upstream:
```
git fetch origin
git rebase origin/main
git push fork main
```

### For new feature work:
1. Sync main first (see above)
2. `git checkout -b feature/xyz`
3. Work on it, push to fork: `git push fork feature/xyz`
4. When ready, merge into main or open a PR to upstream

### Quick sync (no local changes):
```
git pull origin main --ff-only
git push fork main
```

Use `--ff-only` to ensure fast-forward only, never merge commits. Keep history linear.

### Updating feature branches after changes on main

When a commit on `main` relates to an open PR's feature branch, update that branch so the PR stays current. The feature branch should contain **only its own commits** on top of `origin/main` (no unrelated commits like plugin system changes).

```
git checkout feature/xyz
git reset --hard origin/main
git cherry-pick <relevant-commit-hashes>   # only the commits for this feature
cargo build -p tycode-cli                   # verify it builds
git push fork feature/xyz --force-with-lease
git checkout main
```

**Active feature branches & PRs:**
| Branch | PR | Description |
|--------|----|-------------|
| `feature/tui` | #38 | Ratatui-based TUI |
