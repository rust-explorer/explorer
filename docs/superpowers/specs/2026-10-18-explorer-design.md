# Explorer — Design

Date: 2026-10-18
Status: draft, pending owner approval

## 1. Intent

Explorer is a self-hosted web file manager for the owner's custom NAS. It replaces
[filebrowser](https://github.com/filebrowser/filebrowser), which worked well but is no
longer maintained. The owner uses it remotely to browse, upload, download and organise
files and folders through an interface that is intuitive and pleasant to use, with a more
modern look than filebrowser.

It is also a learning project: the owner wants to discover Rust, so the whole product
(backend and frontend) is written in Rust. The owner does not write code; Claude writes it
and the owner reviews every pull request. Explanations of Rust concepts happen in
conversation, never in the repository.

## 2. Goals

- Browse the folders mounted into the container, and only those.
- Upload files up to ~10 GB reliably, resumable after network interruption, including
  whole folder trees dragged or picked from the desktop.
- Download files (resumable) and whole folders (as a streamed zip).
- Manage files and folders: create folder, rename, move, copy, delete, multi-select.
- Secure login through OIDC with the owner's PocketID instance.
- Ship as a single Docker image, easy to install, with no file-permission surprises.
- Code, tickets (GitHub Issues), and documentation all live in
  `git@github.com:rust-explorer/explorer.git`.
- The repository will be public: Explorer must be usable by other self-hosters, and
  publishing it must never expose the owner's private data or NAS layout (see §5.3).

## 3. Non-goals (for v1, but the design must not block them)

- **Multi-user** with per-user permissions. v1 has one user, but user identity flows
  through the whole codebase so per-user authorization can be added without a rewrite.
- **Native mobile app.** v1 is a responsive web app. The frontend framework (Dioxus) was
  chosen so the same UI code can later target iOS/Android.
- **Rich media viewer** (PDF viewer, video player beyond the browser's native one,
  transcoding).
- **Encrypted backups to multiple MEGA accounts** (possibly via rclone). Filesystem access
  is isolated in its own module so a backup feature can reuse it.
- Feature parity with filebrowser. Only the features listed here are wanted.

## 4. Anti-goals (failure even if it "works")

Stated by the owner:

- Visible features that don't work: inactive buttons, half-done screens.
- Bugs without understandable logs.
- Security holes.
- Complex manual configuration at install time.
- File ownership / permission problems on the NAS.

Consequences for the builder:

- A UI control is shipped only when its feature works end to end. No placeholders.
- Every error reaching the UI carries a human-readable message
  (e.g. "Permission denied on /media/Films"); every server error is logged with context
  and a request id.
- Path-safety and auth code is covered by tests before anything else.

## 5. Constraints and decisions

### 5.1 Decided by the owner

| Topic | Decision |
|---|---|
| Language | Rust only, backend and frontend. CSS is the accepted exception; small JS exceptions are tolerated only where unavoidable (e.g. pdf.js later). |
| Backend framework | Axum (on Tokio). |
| Frontend framework | Dioxus (web target in v1, mobile later). |
| Packaging | One Docker image. Only volumes mounted into the container are manageable. |
| Auth | OIDC with PocketID. |
| Reverse proxy | GoDoxy terminates TLS in front of the container. |
| Filesystems | Volumes are local ext4 filesystems (possibly several disks, one per volume). No network or union filesystems need support. |
| Largest files | ~10 GB. |
| Folder upload | Yes: whole folder trees can be uploaded (drag & drop and picker). |
| UI languages | Multilingual from the start. English is the default; French and Spanish are available. |
| UI layout | "Direction A" mockup, approved as-is (see §7). |
| Language of repo | English for code, tickets, docs. |
| Comments | Sparse, as an experienced developer would write. No pedagogical content in the repo (docs excepted). Proven patterns, no hacks. |
| Workflow | One branch + one PR per ticket. The owner reviews and merges every PR. Claude never merges until the owner says otherwise. |
| Git history | Never rewrite the owner's commits (no amend, squash, or rebase folding changes into existing commits). Every change goes in its own commit. |

### 5.2 Public repository constraints (owner)

The owner may make the repository public so others can use Explorer, on two conditions:
it must be genuinely usable by others, and nothing in it may endanger the owner's data
or reveal how their NAS is organised.

- **Nothing personal in the repo, ever**: no real domains, hostnames, IPs, PocketID
  URLs, client ids, secrets, real volume names or real file names. Docs, examples, tests
  and fixtures use `example.com`, generic volume names (`media`, `documents`) and
  synthetic data. `.env`, local compose overrides and `.superpowers/` are git-ignored;
  only a `.env.example` is committed. GitHub secret scanning and push protection are
  enabled.
- **Commit identity**: commits should use the owner's GitHub `noreply` address rather
  than a personal or work email, since history becomes public (owner to confirm).
- **Generic, not tailored**: nothing hard-coded to the owner's setup. OIDC works with any
  compliant provider (PocketID is the documented, tested example); the groups claim name
  is configurable. Defaults are safe for a stranger's install (non-root, deny unless in
  the allowed group, refuse to start on incomplete auth config, no "disable auth" mode).
- **No self-hosted CI runner**: on a public repo it would let fork PRs run code on the
  owner's NAS. GitHub-hosted runners only; workflows triggered by forks get read-only
  tokens and no secrets.
- **License**: non-commercial use only. Use the **PolyForm Noncommercial License 1.0.0**
  (`LICENSE` file, referenced from `README.md` and `Cargo.toml` via `license-file`).
  The README states plainly that the project is source-available for non-commercial
  use, not OSI "open source". Third-party dependencies keep their own licenses (Axum,
  Dioxus, etc. are MIT/Apache-2.0, compatible).
- `SECURITY.md` explains how to report vulnerabilities privately (GitHub private
  vulnerability reporting).
- Switching the repo to public is done by the owner, or by Claude only on the owner's
  explicit go, after checking the history contains nothing private.

### 5.3 Claude's calls (accepted by the owner during brainstorming, still open to change)

- Cargo workspace with three crates: `server` (Axum), `web` (Dioxus), `shared`
  (API types and request/response DTOs used by both sides).
- Clear REST-style JSON API between `web` and `server` rather than Dioxus "fullstack"
  server functions: easier to read, and reusable by a future mobile app or backup tool.
- OIDC handled server-side ("backend for frontend"): `openidconnect` crate, authorization
  code flow with PKCE, state and nonce. Tokens never reach the browser; the browser only
  holds an HttpOnly, Secure, SameSite=Lax session cookie.
- Access is granted to members of a configurable PocketID group (e.g. `explorer-users`).
- SQLite (via `sqlx`) for sessions and small per-user data (favorites).
- Tailwind CSS for styling.
- Strict path confinement (see §8).
- Resumable chunked uploads, preferably implementing the tus 1.0 core protocol
  server-side (creation, HEAD offset, PATCH, termination).
- Documented `docker-compose` example with GoDoxy labels.

## 6. Architecture

```
browser ──HTTPS──> GoDoxy ──HTTP──> explorer container
                                     ├─ Axum server
                                     │   ├─ /api/*      JSON API
                                     │   ├─ /auth/*     OIDC login/callback/logout
                                     │   ├─ /files/*    upload (tus) / download (Range) / zip
                                     │   └─ /*          static Dioxus web bundle (WASM)
                                     ├─ /data/<volume>  mounted volumes (managed)
                                     └─ /config         SQLite DB, thumbnail cache
```

Server internal layering (names indicative):

- `config` — load and validate environment configuration at startup.
- `auth` — OIDC flow, session handling, `CurrentUser` extractor.
- `authz` — authorization policy. v1: "any authenticated member of the allowed group can
  do anything on any volume". Every handler calls through it with the current user, the
  volume and the action, so per-user/per-volume rules can be added later.
- `storage` — all filesystem access: volume discovery, path resolution and confinement,
  listing, metadata, file operations, free space. Handlers never touch `std::fs`/`tokio::fs`
  directly. This module is what a future backup feature reuses.
- `uploads` — resumable upload protocol on top of `storage`.
- `api` — Axum routers/handlers, mapping domain errors to HTTP responses with a stable
  JSON error shape (`code`, `message`, optional `details`).

Frontend: a Dioxus web app that talks to the API through a small typed client using the
`shared` DTOs. Long file lists must render efficiently (virtualised list rendering, i.e.
only visible rows in the DOM).

## 7. User interface

The approved mockup ("Direction A, detailed") is the reference:
`docs/design/mockup-main-screen.html`. Its ✂ icon in the selection bar is the "Move"
action (not a separate cut/paste clipboard); "resumable" status in the uploads panel
reflects §9 behaviour.

Desktop:

- **Top bar**: app name, search field (filename search in the current volume, recursive),
  list/grid toggle, refresh (reloads the current listing), "New folder", "Upload", avatar
  menu (user name, logout).
- **Sidebar**: list of volumes; favorites; free space of the current volume.
- **Main area**: breadcrumb; file table with columns Name / Size / Modified, sortable;
  checkboxes for multi-select. Folders first, then files.
- **Selection bar** when items are selected: download, move, delete (and copy).
- **Context menu** (right click) on an item: download, rename, move to…, copy to…,
  delete (with confirmation). "Preview" appears only once preview is implemented.
- **Grid view**: tiles with thumbnails for common image formats, icons for everything else.
- **Space usage** (added 2026-10-27 at the owner's request): from a folder (or a volume
  root), "Analyse space" scans the tree in the background and opens a screen listing its
  content biggest first, each line with a proportional bar, its size and its share of the
  parent. Clicking a sub-folder goes down into it without a new scan; the screen shows when
  the scan ran and can run it again. A folder's row menu also offers "Calculate size",
  which runs the same scan and shows the result in the Size column.
- **Drag & drop** files from the desktop anywhere on the listing to upload them into the
  current folder.
- **Floating uploads panel**: per-file progress, speed, cancel; collapsible; the user can
  keep navigating while uploads run.
- **Theme**: light/dark following the OS setting.
- **Language**: UI strings come from translation files; English by default, French
  and Spanish available, chosen from the browser language with a manual override in the avatar
  menu. Dates, sizes and numbers are formatted for the active locale. The mockup's French
  wording is illustrative only.

Mobile (responsive web):

- Sidebar moves into a ☰ menu; search is an icon in the top bar.
- Each row shows name, size and date; a ⋯ button opens the actions menu.
- Floating "+" button for Upload / New folder.

Errors are shown as non-blocking toasts with the server's message. Destructive actions
(delete, overwrite) always ask for confirmation.

## 8. Security

- **Path confinement** is the most important property. Every client-supplied path is
  `(volume_id, relative_path)`. Resolution rejects absolute paths, `..` components,
  NUL bytes, and any symlink whose target resolves outside the volume root. Resolution
  must be robust against TOCTOU tricks where practical (e.g. resolving with
  `openat`-style APIs such as `cap-std`, or canonicalising and re-checking the prefix
  immediately before each operation). The builder chooses the mechanism; it must be
  covered by thorough tests (traversal, encoded traversal, symlink escape, symlink inside
  the volume, unicode names).
- **Authentication**: OIDC authorization code + PKCE + state + nonce; ID token signature,
  issuer, audience and expiry validated. Group membership read from the `groups` claim
  (PocketID supports the `groups` scope); non-members get a clear "access denied" page.
- **Sessions**: HttpOnly, Secure, SameSite=Lax cookie; session id rotated at login;
  server-side expiry (sliding, default 7 days idle, configurable); logout destroys the
  session server-side and redirects to PocketID's end-session endpoint if advertised.
- **CSRF**: all state-changing requests require a valid `Origin` (or `Referer`) matching
  the configured public URL, on top of SameSite cookies.
- **No secrets in logs** (tokens, cookies, client secret).
- Standard security headers (CSP suited to a WASM app, `X-Content-Type-Options`,
  `Referrer-Policy`, frame-ancestors none). Downloads use `Content-Disposition` correctly
  so user files are never rendered as HTML in the app's origin.
- The container runs as a non-root user.

## 9. Files and data handling

- **Volumes**: every directory directly under `/data` is a volume; its name is the
  directory name. Discovered at startup. At startup each volume is checked for
  readability and writability; a read-only volume is shown as read-only in the UI (write
  actions hidden/disabled with an explanation) and logged clearly.
- **Ownership**: the container process runs as the UID/GID given by Docker's `user:`
  directive (documented, default `1000:1000`), so created files belong to the owner's
  NAS user. No chown magic. Startup logs the effective UID/GID and warns explicitly when
  a volume is not writable by it.
- **Uploads**: chunked and resumable (target chunk size 10–50 MB, builder's choice).
  Data is streamed to disk, never buffered whole in memory. In-progress uploads live in a
  hidden staging directory on the same volume (e.g. `/data/<volume>/.explorer-uploads/`),
  hidden from listings, and are atomically renamed into place when complete. Abandoned
  uploads are cleaned up after 24 h. The client retries automatically after network
  errors; if the page is reloaded, re-selecting the same file resumes it (matched by
  name + size + last-modified).
- **Folder uploads**: each file carries its path relative to the dropped/picked folder;
  the server creates missing directories (confined like any other path). The resume key
  includes the relative path. Empty directories are created too.
- **Name conflicts** (upload, move, copy, rename): the user chooses Replace, Keep both
  (auto-suffix ` (1)`), or Cancel, with an "apply to all" option for multi-item
  operations and folder uploads.
- **Downloads**: streamed, with HTTP Range support (resume, and seeking in media later).
- **Folder download**: zip generated on the fly and streamed (stored, not compressed),
  no temporary file.
- **Move/copy of large data** must not depend on a single long HTTP request: run as
  server-side tasks whose progress is visible in the same floating panel as uploads.
  Mechanism (polling vs. SSE/WebSocket) is the builder's call.
- **Cross-volume moves**: volumes may be different disks, so a move between volumes is a
  copy followed by delete (rename only within a volume; fall back on `EXDEV`).
- **Delete** is permanent after confirmation (no trash in v1).
- **Thumbnails**: generated server-side for common image formats with a pure-Rust
  decoder, cached under `/config/cache`, invalidated by mtime/size.
- **Search**: case-insensitive filename substring search, recursive within the current
  volume, with a result cap and cancellation; results show their parent path.
- **Favorites**: stored per user (OIDC `sub`) in SQLite.
- **Space usage scans** (added 2026-10-27) run as background tasks like move/copy, with
  progress and cancel in the same panel. They walk the tree with the server's own confined
  filesystem code in parallel (no external `du`, which the image does not ship), and
  follow `du`'s rules: allocated size (`st_blocks × 512`), hard links counted once, no
  crossing into other filesystems, symlinks not followed. Results are kept in memory for a
  limited time, per user; a restart loses them.

## 10. Configuration and installation

Goal: a user with an OIDC provider (e.g. PocketID) and Docker installs Explorer in minutes.

Environment variables (names indicative, prefix `EXPLORER_`):

| Variable | Required | Purpose |
|---|---|---|
| `EXPLORER_PUBLIC_URL` | yes | External URL (e.g. `https://files.example.com`), used for redirect URI and Origin checks |
| `EXPLORER_OIDC_ISSUER` | yes | OIDC issuer URL (e.g. PocketID) |
| `EXPLORER_OIDC_CLIENT_ID` | yes | |
| `EXPLORER_OIDC_CLIENT_SECRET` | yes | (also readable from a file, `*_FILE`, for Docker secrets) |
| `EXPLORER_ALLOWED_GROUP` | yes | Group allowed to log in allowed to log in |
| `EXPLORER_OIDC_GROUPS_CLAIM` | no | Claim holding group names, default `groups` |
| `EXPLORER_SESSION_TTL` | no | Idle session lifetime, default 7d |
| `EXPLORER_LOG` / `EXPLORER_LOG_FORMAT` | no | Log level filter; `pretty` or `json` |

Fixed paths: `/data` (volumes), `/config` (state). Listening port: 8080.

Startup validates everything and exits with a single explicit message per problem
(missing variable, unreachable issuer, `/config` not writable…). A `/healthz` endpoint
supports the Docker healthcheck.

## 11. Observability

- `tracing` with a request-id per HTTP request (propagated in an `X-Request-Id`
  response header and shown in UI error toasts so the owner can grep logs).
- Every file operation logs user, action, volume and path at `info`; failures at `warn`/
  `error` with the underlying OS error.

## 12. Quality, CI and delivery

- Tests: unit tests for `storage` (path confinement first), `authz`, config; integration
  tests for the API against a temp directory and a fake/mock OIDC issuer. Frontend logic
  kept thin and tested where it has logic (e.g. upload chunking/resume state).
- **CI budget constraint (owner):** the owner is on a free GitHub plan and will not pay
  for this project. CI must never block work for lack of minutes, and must be bypassable:
  - A single local entry point (e.g. `just ci` or `cargo xtask ci`) runs exactly what CI
    runs (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, Docker build).
    CI workflows call that same entry point. Claude runs it locally before every PR.
  - CI checks are informative, not required: branch protection must not make merges
    depend on a green CI run.
  - Minutes are spent sparingly: aggressive caching (`Swatinem/rust-cache`), path filters
    (docs-only changes skip CI), concurrency groups cancelling superseded runs, and the
    expensive Docker image build only on version tags or manual dispatch.
  - Once the repository is public, GitHub-hosted runner minutes are free and unlimited.
    While it is private, the free plan's default $0 spending limit means exhausted minutes
    stop jobs rather than incur charges. No self-hosted runner (see §5.2).
- On a version tag: `linux/amd64` image published to GHCR
  (`ghcr.io/rust-explorer/explorer`). The owner's NAS is Intel; arm64 is not built (emulated
  arm64 builds are very slow) but nothing should prevent adding it later.
- Multi-stage Dockerfile: build web bundle with the Dioxus CLI, build the server, copy
  both into a minimal runtime image.

## 13. Documentation (in repo)

- `README.md` — what it is, screenshot, quick start.
- `docs/installation.md` — PocketID client setup (redirect URI, groups scope),
  `docker-compose.yml` with GoDoxy labels, volumes, `user:` UID/GID.
- `docs/configuration.md` — every variable.
- `docs/architecture.md` — crates, modules, request flow.
- `docs/security.md` — threat model, path confinement, auth/session/CSRF.
- `LICENSE` (PolyForm Noncommercial 1.0.0), `SECURITY.md`, `.env.example`.

## 14. Delivery order (milestones → GitHub milestones and issues)

1. **Foundation** — workspace, CI, Dockerfile, config loading, logging, health check,
   static serving of an empty Dioxus app, i18n layer (en, fr, es), docs skeleton.
2. **Authentication** — PocketID OIDC login/logout, sessions in SQLite, group check,
   `CurrentUser` + `authz` skeleton, CSRF protection.
3. **Browse & download** — volume discovery, path confinement, listing, breadcrumb,
   sorting, file download with Range, folder zip download, sidebar with free space.
4. **Upload** — resumable chunked upload of files and folder trees, drag & drop, floating
   progress panel, conflict handling.
5. **Manage** — new folder, rename, move, copy, delete, multi-select, context menu,
   background tasks for long operations.
6. **Polish** — grid view with thumbnails, search, favorites, dark mode, mobile layout
   refinements, read-only volume UX; refresh and space usage (added 2026-10-27).
7. **v1.0 release** — docs complete, tagged image on GHCR.

Each milestone ends with something usable; no UI element from a later milestone appears
earlier.

Ticket workflow (Claude's call): Claude creates the GitHub milestones and issues from
the plan. The repository exists and is empty. Tickets are small and ordered so that each
PR branches from an up-to-date `main`; when a ticket depends on an unmerged PR, its branch
may start from that PR's branch, and is brought up to date by merging (never rebasing)
once the parent is merged.

## 15. Left to the builder

- Exact crate choices beyond those decided (tus implementation, zip streaming, path
  confinement mechanism, session store, thumbnail library, virtual list approach).
- API route shapes and DTO details.
- Progress-reporting mechanism for background tasks.
- Precise visual design (colours, spacing, icons) within the approved layout.
- Test tooling for the frontend.
