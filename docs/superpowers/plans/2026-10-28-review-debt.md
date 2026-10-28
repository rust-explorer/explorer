# Review Debt Implementation Plan

> **For agentic workers:** work item by item; each item is a small, independent fix with its own test where one can be written.

**Goal:** Close the minor findings that the M4, M5 and M6 reviews left open, before the v1.0 release work (M7).

**Architecture:** No new feature and no new dependency. Each fix lands in the domain that owns the code. Two files over the ~400-line rule are split by responsibility (`web/src/uploads/queue/mod.rs`, `web/src/tasks/store.rs`).

**Spec:** `docs/superpowers/specs/2026-10-18-explorer-design.md`. The owner accepted every decision left as built in M5 and M6 (2026-10-28); none of them is reopened here.

## Global Constraints

- Everything from the M1–M6 plans' Global Constraints still applies (English, sparse comments, no commit rewriting, all filesystem access through `storage`, blocking calls in `spawn_blocking`, no `document::eval`, inline unit tests, domain folders, en/fr/es strings).
- **Branch** `debt/review-followups` from `main`. **One commit per domain** (for example `fix(uploads): …`, `fix(tasks): …`); a fix after review is its own commit.
- An item that turns out to be already fixed, or not a defect once read in context, is skipped and listed as such in the PR body with one line of why.
- `cargo xtask ci` passes, and the Linux CI passes, before the PR is called ready.

## Server and shared

**Uploads**
- A commit that fails leaves the folders it created: remove the folders this commit created (deepest first, only if still empty).
- A dangling symlink in the middle of the target path is only reported (`AlreadyExists`) after the whole upload: check the parent path when the upload is created.
- A fatal upload failure leaves the staged file until the 24 h sweep: delete it when the upload is marked failed.
- A cancel that arrives during the last chunk can still commit: re-check the cancelled flag under the lock right before commit.
- `commit_staged` errors report the volume root instead of the target path.
- Missing test: exclusive creation of `.explorer-uploads`.
- A symlink or hard link pointing straight at a staged file is not refused at commit.
- No fsync of the destination folder after commit.

**Storage and tasks**
- `place()` with Replace and nothing at the destination uses a plain `renameat` (empty-folder race) and swallows metadata errors.
- `StorageError::InvalidPath` is not logged at warn.
- Filesystems without `RENAME_NOREPLACE` report `invalid_destination` on same-volume moves: fall back to a safe link/exclusive path or report a clear error.
- No per-user cap on queued tasks.
- Per-batch conflict decisions are never removed.
- Task totals are set in three places.
- The root guard is duplicated in `move.rs`.
- Missing tests: delete-phase failure of a cross-volume move; picker start folder (server side if any).
- Only `Io` errors carry the OS errno: keep the source in `StorageError` for diagnostics.

**Downloads**
- A duplicate `name=` in a zip request is not deduplicated.
- The zip `send_timeout` polls every 2 ms.
- Folder dates (1980) and UTC times in zips.
- Range edge cases: `0-1,-0`, `100-50`, `-5` on an empty file.
- `Sec-Fetch-Site` check on `/files/*` (a cross-site GET can start a zip).

**Search, favorites, thumbnails, usage (M6)**
- Search cancel and deadline are only checked between folders: also check inside a large folder.
- `MIN_QUERY_CHARS` is duplicated in web and server: move it to `shared`.
- No unit test for the search drop guard.
- `DELETE /api/favorites` does not normalise the path, and logs even when nothing was removed.
- Thumbnails: the temp file is not fsynced before rename; `If-None-Match: *` answers 304; unreadable images log a warn per tile (debug instead); `Capped::seek(End)` uses the capped length instead of the real one.
- Usage: open folder fds grow with depth; the hard-link set is unbounded during a scan; non-UTF-8 names are converted lossily.

## Web

**Uploads**
- An early wake of the timer can skip a refresh or a retry.
- "Couldn't be read" also counts invalid names.
- Retry wording says "Connection lost" for 423, 409 and 500.
- `error-permission_denied` says "open" in the upload context.
- A non-JSON 413 from a proxy shows a generic error.
- `aria-controls` points to ids that are not mounted.
- Upload, task and rename conflict prompts can still stack in edge cases.
- Split `uploads/queue/mod.rs` (over 1100 lines).

**Manage**
- The menu clamp uses an estimated height: measure the menu.
- The select-all checkbox shifts when the selection bar shows.
- The selection retain effect writes the signal on every refresh even when nothing changed.
- Cancel stays enabled while a dialog request is busy.
- The new-folder error is not cleared on input.
- The result of the cancel `DELETE` is discarded.
- Polling stops after sign-out and leaves rows Waiting.
- The picker label points to a missing select while loading.
- The focus request lingers if a renamed entry never appears.
- Split `tasks/store.rs` (over 900 lines).

**Polish (M6)**
- "100" is hardcoded in the favorites-full message (use a Fluent variable); a failed favorites load is silent.
- Duplicate `aria-label` on the grid region and list; 404 round-trips for heic/svg/avif tiles (skip types the server cannot render).
- Adopted scans show the first event after reload as "calculated at"; dismissed usage tasks stay in the tab list; the retention check is blind to other tabs.
- "Scan finished" may be announced again across sub-folders.
- Folder rows re-render on each poll.
- "No longer available" flashes on reload.
- Refresh shows a hover state while `aria-disabled`.
- The dark-mode lint checks per colour family only.
- The `color-scheme` meta is set after boot (flash).
- The read-only notice shifts the layout; it says "use Upload" on phones where the control is "+".
- Typing only spaces in the search field wipes the listing.
- Fluent bidi isolation marks inside `aria-label`s.
