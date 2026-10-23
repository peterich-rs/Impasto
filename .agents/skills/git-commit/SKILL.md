---
name: git-commit
description: Plan and write commits from the unpublished branch delta, not only the working tree. Use when the user asks to commit, amend, squash, or tidy local history. Messages use conventional type(scope), a bullet body.
---

# Git Commit

The result is a branch history a reviewer can read. Each commit is one change, with a clear boundary. Decide that from the unpublished delta against a baseline, including commits already made and not pushed. The working tree alone is not the unit of work.

A series of "edit, commit, edit again" steps is often still one or two changes. Fold those drafts together instead of stacking another tweak commit.

## Baseline

The baseline is the first commit that must stay as it is. Everything after it, plus the working tree, is the draft you may reshape.

1. If `@{u}` is set, the baseline is `@{u}`.
2. If `@{u}` is missing but `origin/<current-branch>` exists, the baseline is that remote tip. An unset upstream does not mean the branch was never pushed.
3. Otherwise the baseline is the integration branch this work came from: `origin/develop` if it exists, else `develop`.

Do not use `develop` once this branch has a remote tip. Commits already on that tip are published. Leave them unless the user explicitly asks to rewrite published history.

If local HEAD and the remote tip have diverged (`git rev-list --left-right --count <baseline>...HEAD` shows commits on both sides), stop. Do not reset across a divergence.

## Read the draft as one pile

Run these before choosing a message or `git add`:

```bash
git status -sb
git rev-parse --abbrev-ref HEAD
git log --oneline <baseline>..HEAD
git diff --stat <baseline>
```

`git diff <baseline>` is the working tree against the baseline, so it includes unpublished commits and unstaged edits. Untracked files appear only in `git status`. Read that whole delta, and the existing messages in `<baseline>..HEAD`, before deciding how many commits the work actually is.

## Cut the commits

Group the pile by concern. A concern is one reviewable change: one bug, one feature, one behavior. Name the concerns first, then assign paths.

- Several local commits and a later edit that continue the same concern become one commit.
- Two concerns stay two commits, in the order a reader needs. Do not squash unrelated work just to shorten history.
- An unpublished commit that is already one finished concern stays. Fold a follow-up into it when the follow-up is the same concern. Add a new commit only when the follow-up is a different concern.
- Do not split by directory, by file count, or by tag. A small change that touches several modules is still one commit. Split a large change by module only when those modules are independent concerns.
- `okhttp` and its `kyc` mirror are one concern. Keep them in the same commit and add the bullet `- Mirror all changes to kyc module`.

## Rewrite the unpublished range

Published commits stay put. Unpublished commits are drafts.

- The new edit belongs on the tip, and the tip is unpublished and already the right concern: stage those paths and `git commit --amend`.
- The unpublished range plus the working tree is one concern, but history is several partial commits: `git reset --soft <baseline>`, then make one commit. Do this only when every commit in the range is unpublished. After the reset, those commits are staged and any extra working-tree edits are still unstaged. Stage those paths too, or the new commit drops them.
- The pile is two concerns and the current commits mix them: `git reset --soft <baseline>`, then stage and commit one concern at a time. Stage explicit paths for each commit, including the unstaged edits that belong to it. Do not `git add -A` across concerns.
- Existing unpublished commits are already a clean story, and the working tree is a new concern: commit only the new paths. Leave the earlier commits.

After rewriting, `git log --oneline <baseline>..HEAD` should name the concerns, and `git diff --stat <baseline>` should match that list. The working tree for those paths should be clean.

Do not `git push`, and do not force-push, unless the user asks. A rewritten local history is still local.

## Workflow

1. Resolve the baseline with the rules above.
2. Read `git status -sb`, `git log --oneline <baseline>..HEAD`, and `git diff --stat <baseline>`.
3. List the concerns in that pile. Decide which unpublished commits to keep, fold, or replace.
4. Rewrite or commit as in the section above. Exclude secrets and credentials.
5. Write each message with the format below.
6. Run `git log -1 --format=%B`. If a forbidden attribution trailer is present, rewrite the message without it.
7. Run `git status` and `git log --oneline <baseline>..HEAD`. Confirm the story matches the concerns.

## Message Format

```
type(scope): concise summary

- component/method: what changed and why
- component/method: what changed and why
```

### Subject

- **type**: `feat`, `fix`, `chore`, `refactor`, `docs`
- **scope**: module name, such as `okhttp`, `wehttp`, `kyc`, `httpdns`. Omit it when the change crosses modules.
- **summary**: lowercase start, imperative, no period. The whole subject, including the tag, stays under 72 characters.

### Body

- Each bullet is `- `, then the component, then colon, then what changed and why.
- Name the class, method, or file. Skip narration of the edit itself.
- When `okhttp` and `kyc` both change, add `- Mirror all changes to kyc module`.

### Forbidden attribution

The author is only the repo git user. Never add, keep, or restore an AI or tool credit.

Reject all of these, including when a hook or wrapper appends them:

- `Co-authored-by:` for Cursor, Claude, Copilot, GPT, Codex, or similar tools
- `Made-with`, `Generated-by`, `Assisted-by`, or any other AI attribution trailer

After every `git commit` or `git commit --amend`, read `git log -1 --format=%B`. If any of those lines are present, rewrite the message without them.

### Commit command

Stage paths, then:

```bash
git commit -m "$(cat <<'EOF'
type(scope): summary

- detail 1
- detail 2
EOF
)"
```

## Examples

**One concern that arrived as several drafts.** Unpublished commits "add manifest", "document manifest", and "fix manifest host", plus an unstaged icon-path edit, are still one change. Soft-reset to the baseline and make one commit:

```
fix(ci): point the release manifest at the env static host

- publish.sh: write deployDomain, packageUrl, and icons for that env
- build.sh: pack the icon beside the manifest
- README: list the manifest, package, and icon urls
```

**A second concern shows up later.** The unpublished manifest commit is already one change. The working tree only changes the phone-auth app id. Leave the manifest commit and add:

```
fix(sdk): give stg its own sdk app id

- SdkConfig.appId: stg uses its own app id
- README: document the per-flavor app id
```

**Code and the short doc that explains it:**

```
fix(ci): pack the release archive with its manifest 

- build.sh: write the manifest into the archive
- README: document the archive layout next to that script change
```

**Non-code blob, separate because of its size:**

```
chore(httpdns): update httpdns native binaries

- Update arm64-v8a/armeabi-v7a libwb_httpdns.so to latest build
```

**Version bump:**

```
chore: v2.2.18
```
