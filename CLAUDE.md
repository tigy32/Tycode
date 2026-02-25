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
