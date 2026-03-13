# DogeOS Fork Workflow

This fork stays maintainable by treating Scroll's `master` as the source of
truth and keeping DogeOS work as a thin overlay.

## Branch model

- `master`: exact mirror of `upstream/master`. Do not add DogeOS commits here.
- `dogeos/main`: canonical DogeOS base branch. Keep it small, linear, and easy
  to rebase.
- `dogeos/<feature>`: feature overlays opened as PRs into `dogeos/main`.

## Update workflow

1. `git fetch --all --prune`
2. `git checkout master`
3. `git merge --ff-only upstream/master`
4. `git push origin master`
5. `git checkout dogeos/main`
6. `git rebase master`
7. `git push --force-with-lease origin dogeos/main`
8. `git checkout dogeos/<feature>`
9. `git rebase dogeos/main`
10. `git push --force-with-lease origin dogeos/<feature>`

## Patch policy

- Prefer one-purpose commits that can be cherry-picked or dropped cleanly.
- Keep DogeOS-only documentation under `dogeos/`.
- Keep code changes small and easy to compare with upstream.
- If a feature needs dedicated fixtures, place them under
  `testdata/dogeos/<feature>/`.
- If a feature needs dedicated tests, keep them narrow and name them after the
  feature they cover.
