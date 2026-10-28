# M3 Browse & Download Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A signed-in user browses the volumes mounted under `/data`: sidebar with volumes and free space, breadcrumb, sortable file table that stays fast on huge folders, download of any file (resumable, Range) and of any folder as a streamed zip.

**Architecture:** A new `storage` module in `crates/server` owns all filesystem access. Each volume is a `cap_std::fs::Dir` capability, so every path is resolved beneath the volume root by the OS (`openat2(RESOLVE_BENEATH)` on Linux) and a symlink can never escape. Handlers (`api/browse.rs`, `files/download.rs`, `files/zip.rs`) only talk to `storage` through `authz::Policy`. The web app gets the real main screen of the approved mockup, minus every control that belongs to a later milestone.

**Tech Stack:** Existing M2 stack, plus cap-std 4.0, rustix `fs` feature (statvfs, access), http-range-header 0.4, tokio-util 0.7 (`io`), zip 9.0 (stream writer), percent-encoding 2.3, httpdate 1.0 (server); js-sys 0.3 (web, `Intl.DateTimeFormat`).

**Spec:** `docs/superpowers/specs/2026-10-18-explorer-design.md` (milestone 3 of §14; §6, §7, §8 path confinement and downloads, §9 volumes/downloads/folder download, §11).

## Global Constraints

- Everything from the M1 and M2 plans' Global Constraints still applies: English, sparse comments, no commit rewriting, one stacked branch + PR per task (`m3/<slug>`, the first one based on `m2/docs-ci` because M2 is not merged yet), owner merges, no personal data, `cargo xtask ci` passes before each PR, no UI control that does not work end to end.
- No UI element from a later milestone: no search field, no list/grid toggle, no New folder, no Upload, no checkboxes or selection bar, no context menu, no favorites section.
- Client paths are always `(volume, path)`: `volume` is a volume name, `path` is relative, `/`-separated, without leading `/`, `""` meaning the volume root. The server never accepts an absolute or host path.
- All filesystem access goes through `storage`; no `std::fs`/`tokio::fs` on user data outside it. Blocking `cap-std` calls run in `tokio::task::spawn_blocking`.
- Volume = each real directory (not a symlink) directly under `config.data_dir`, discovered once at startup, sorted by name. Names that are not valid UTF-8 are skipped with a `warn`.
- Reserved server prefixes: `/api`, `/auth`, `/files`, `/assets`, `/healthz`. Every unknown path under `/api`, `/auth`, `/files` gets the JSON 404. SPA routes: `/browse/<volume>/<path…>`, `/access-denied`, `/signin-error`, `/` (redirect to the first volume).
- New JSON error codes (in `ErrorBody.code`, with `details: {"volume": .., "path": ..}` when they concern a path): `volume_not_found` 404, `path_not_found` 404, `invalid_path` 400, `not_a_directory` 400, `is_a_directory` 400, `permission_denied` 403, `outside_volume` 403, `range_not_satisfiable` 416. Messages are English and name the path, e.g. `Permission denied on media/Films`. The UI translates by code and falls back to the server message.
- User files are never rendered by the browser in the app's origin: downloads always send `Content-Type: application/octet-stream`, `Content-Disposition: attachment`, `Content-Security-Policy: sandbox`, `X-Content-Type-Options: nosniff`.
- Logging: downloads and zips at `info` with `user` (OIDC `sub`), `action`, `volume`, `path`; listings at `debug`; storage failures at `warn` with the OS error; `outside_volume` at `warn` (security event). Never log file contents.
- Sizes use SI units (1 kB = 1000 B) and locale-specific unit labels and decimal separators (en/es `B kB MB GB TB`, fr `o ko Mo Go To`; one decimal below 10, otherwise none: `9.8 GB`, `82 kB`, `1,2 Go`, `532 Mo`).
- Dynamic sizes in the DOM (virtual list spacers, free-space bar) are set only through Dioxus style properties (`height: "{h}px"`), which go through CSSOM and are allowed by the CSP. Never a `style: "…"` string attribute, never `document::eval`.

## Review Focus

1. **Traversal through the HTTP layer, not just the parser.** `path=../etc`, `path=%2e%2e/x`, `path=a/../../x`, `path=/etc/passwd`, `path=a%00b`, a volume named `..` in the URL must give `invalid_path` (or `volume_not_found`), never a file outside the volume. Tests in Task 4 and Task 5.
2. **Symlink inside a volume pointing outside it** (`media/escape -> /etc`) and **pointing to another volume.** It is listed as kind `other`, listing into it, downloading it and zipping a folder containing it never reads outside the volume. Tests in Task 3, Task 5 and Task 6.
3. **Names with spaces, `#`, `?`, `%`, accents and emoji** must round-trip through SPA URLs, the API query string and download links. Tests in Task 4 (API) and Task 8 (route parse/format, link building).
4. **Resuming a 10 GB download.** `Range: bytes=N-` gives 206 with the right `Content-Range`; an `If-Range` that no longer matches (file replaced) gives the full 200, not a corrupt splice. Test in Task 5.
5. **A folder of 20 000 entries.** The listing renders in under a second and only the visible rows exist in the DOM. Host test for the windowing math in Task 8, manual check in Task 9.

---

### Task 1: Housekeeping and route reservation

**Files:**
- Modify: `crates/server/src/telemetry.rs`, `crates/server/src/csrf.rs`, `crates/server/src/auth.rs`, `crates/server/src/web.rs`, `crates/server/src/app.rs`, `crates/server/src/error.rs`, `crates/server/tests/static_files.rs`, `crates/server/tests/health.rs`

**Interfaces:**
- Produces:
  ```rust
  // telemetry.rs: replaces the private copies in csrf.rs and auth.rs
  pub fn request_id(headers: &HeaderMap) -> &str;
  pub fn truncate_for_log(value: &str) -> String; // MAX_LOGGED_LEN chars, lossless for valid UTF-8
  // error.rs
  impl ApiError {
      pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self;
      #[must_use] pub fn with_details(self, details: serde_json::Value) -> Self; // copied into ErrorBody.details
  }
  ```
  - The global CSP becomes `SetResponseHeaderLayer::if_not_present`, so a handler can set its own. The other security headers stay `overriding`.
  - `/files` is nested like `/api` with the JSON 404 fallback, plus the bare `/files/` route.

- [ ] **Step 1: Write failing tests**
  - `static_files.rs`: `missing_asset_is_bare_404`. `GET /assets/nope-123.js` → 404, the body is not HTML.
  - `static_files.rs`: `files_prefix_is_reserved`. `GET /files/unknown` and `GET /files/` → 404 JSON `code == "not_found"`.
  - `health.rs`: `handler_csp_is_kept`. A test route returning `Content-Security-Policy: sandbox` → the response has exactly `sandbox`. A normal route still gets the global CSP.
  - `health.rs`: `error_details_are_serialised`. A test route returning `ApiError::new(StatusCode::BAD_REQUEST, "invalid_path", "x").with_details(json!({"volume":"media"}))` → body `details.volume == "media"` and `request_id` set.
- [ ] **Step 2:** Run `cargo test -p explorer-server` → the new tests FAIL.
- [ ] **Step 3:** Implement. `csrf.rs` and `auth.rs` use the telemetry helpers; their existing tests move with the helpers' behaviour.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `chore(server): shared log helpers, per-response csp, reserve /files`.

### Task 2: Volume discovery and relative paths

**Files:**
- Create: `crates/server/src/storage/mod.rs`, `crates/server/src/storage/path.rs`, `crates/server/src/storage/volumes.rs`, `crates/server/tests/volumes.rs`
- Modify: `crates/server/src/lib.rs`, `crates/server/src/app.rs` (`AppState` gains `volumes: Arc<Volumes>`), `crates/server/src/main.rs`, `crates/server/tests/common/mod.rs` (helpers take a data dir), Cargo manifests (`cap-std`, rustix `fs`)

**Interfaces:**
- Consumes: `Config::data_dir`.
- Produces:
  ```rust
  // storage/path.rs
  #[derive(Debug, Clone, PartialEq, Eq)] pub struct RelPath(/* validated components */);
  impl RelPath {
      pub fn parse(raw: &str) -> Result<Self, InvalidPath>; // "" = root
      pub fn root() -> Self;
      pub fn as_str(&self) -> &str;           // "a/b", "" for root
      pub fn file_name(&self) -> Option<&str>;
      pub fn is_root(&self) -> bool;
  }
  #[derive(Debug, thiserror::Error)] #[error("invalid path: {reason}")] pub struct InvalidPath { pub reason: &'static str }
  // storage/volumes.rs
  pub struct Volume { /* name, cap_std::fs::Dir root, read_only: bool */ }
  impl Volume { pub fn name(&self) -> &str; pub fn read_only(&self) -> bool; }
  pub struct Volumes { /* Vec<Volume> sorted by name */ }
  impl Volumes {
      pub fn discover(data_dir: &Path) -> io::Result<Self>; // Err only when data_dir itself is unreadable
      pub fn get(&self, name: &str) -> Option<&Volume>;
      pub fn iter(&self) -> impl Iterator<Item = &Volume>;
  }
  ```
  - `RelPath::parse` splits on `/` and drops empty components, so `a//b/` becomes `a/b`.
  - It rejects (`InvalidPath`): a leading `/`; `.` and `..` components; NUL; any control character; total length over 4096 bytes; any component over 255 bytes.
  - Backslashes are ordinary characters (valid in ext4 names).
  - `read_only` comes from `rustix::fs::accessat(root, ".", Access::WRITE_OK, AtFlags::EACCESS)`. Explorer never writes into a volume to test it.
  - `main` calls `discover` after the database. On `Err`, it logs one line naming the directory and exits 2.
  - Startup logging:
    - `info` per volume, with `volume` and `read_only`;
    - `warn` per read-only volume: `volume {name} is not writable by uid {uid} gid {gid}; check the mount and the container user:`;
    - `warn` when no volume was found: `no volumes found under {data_dir}; mount folders as /data/<name>`.

- [ ] **Step 1: Write failing tests**
  - Unit tests in `path.rs`, `rel_path_accepts_and_normalises`: `""` → root; `"a/b"`; `"a//b/"` → `"a/b"`; `"Films/Dune Part Two.mkv"`; `"é/😀 #?%.txt"`; `"a\\b"`.
  - `rel_path_rejects`: each of `"/etc"`, `".."`, `"a/../b"`, `"./a"`, `"a\0b"`, `"a\nb"`, a 4097-byte path and a 256-byte component gives `Err`.
  - `tests/volumes.rs`, `discovers_directories_sorted`: a tempdir containing `media/`, `documents/`, a file `notes.txt` and a symlink `link -> media` → names `["documents", "media"]`.
  - `read_only_volume_is_detected` (unix, skipped when root): a 0o555 volume → `read_only() == true`; a normal one → `false`.
  - `empty_data_dir_is_ok` → zero volumes.
  - `missing_data_dir_is_error`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement and wire it into `AppState` and `main`. The test helpers create `<tempdir>/data/media` by default.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): volume discovery and validated relative paths`.

### Task 3: Confined filesystem operations

**Files:**
- Create: `crates/server/src/storage/ops.rs`, `crates/server/src/storage/error.rs`, `crates/server/tests/storage.rs`
- Modify: `crates/server/src/storage/mod.rs`, `crates/server/src/storage/volumes.rs`

**Interfaces:**
- Consumes: `Volume`, `RelPath` (Task 2).
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum EntryKind { Dir, File, Other }
  #[derive(Debug, Clone)] pub struct EntryMeta { pub name: String, pub kind: EntryKind, pub size: Option<u64>, pub modified: Option<SystemTime> }
  pub struct Space { pub total: u64, pub available: u64 }
  pub enum WalkItem { Dir(RelPath), File { path: RelPath, size: u64, modified: Option<SystemTime> } }
  #[derive(Debug, thiserror::Error)] pub enum StorageError {
      NotFound(RelPath), NotADirectory(RelPath), IsADirectory(RelPath),
      PermissionDenied(RelPath), OutsideVolume(RelPath), Io { path: RelPath, source: io::Error },
  }
  impl Volume { // all blocking; callers wrap in spawn_blocking
      pub fn list(&self, dir: &RelPath) -> Result<Vec<EntryMeta>, StorageError>; // sorted by name (bytewise)
      pub fn stat(&self, path: &RelPath) -> Result<EntryMeta, StorageError>;
      pub fn open_file(&self, path: &RelPath) -> Result<(std::fs::File, EntryMeta), StorageError>;
      pub fn walk(&self, dir: &RelPath) -> Result<Vec<WalkItem>, StorageError>; // depth-first, dirs before their contents
      pub fn space(&self) -> io::Result<Space>; // rustix statvfs: f_blocks*f_frsize, f_bavail*f_frsize
  }
  ```
  How each operation behaves:
  - **Symlinks.** Entries are resolved through `Dir` with symlinks followed beneath the root.
    - A symlink whose target is inside the volume reports its target's kind.
    - A broken symlink, an escaping one, or anything that is neither a file nor a dir is `Other`, with `size: None`.
  - **`list`.**
    - An entry whose name is not UTF-8 is `Other`, with its name converted lossily.
    - Reading the directory itself fails with the mapped error.
  - **`walk`.**
    - Skips `Other` entries and logs each one at `debug`.
    - Fails on the first unreadable directory with `PermissionDenied(that dir)`.
  - **Error mapping.** An OS error maps from `io::ErrorKind`:
    - `NotFound` → `NotFound`;
    - `PermissionDenied` → `PermissionDenied`;
    - `NotADirectory` → `NotADirectory`;
    - `IsADirectory` → `IsADirectory`;
    - cap-std's escape error ("a path led outside of the filesystem") → `OutsideVolume`;
    - anything else → `Io`.

- [ ] **Step 1: Write failing tests** in `tests/storage.rs`, against a tempdir volume:
  - `lists_files_and_dirs_with_metadata`: size and mtime are correct.
  - `symlink_inside_volume_is_followed`: `photos/latest -> 2026/a.jpg` → kind `File`, size of the target.
  - `symlink_outside_volume_is_other_and_unreadable`: `escape -> <another tempdir>/secret.txt` and `escape_dir -> <another tempdir>` are listed as `Other`. Then `stat`, `open_file`, `list` on `escape_dir` and `list` on `escape_dir/x` all give `OutsideVolume` (or `NotFound` for the child), and never read the secret (Review Focus 2).
  - `symlink_to_sibling_volume_is_outside`: `media/docs -> ../documents` → `OutsideVolume`.
  - `broken_symlink_is_other`
  - `unicode_names_round_trip`: `é/😀 #?%.txt` is listed, stat'ed and opened.
  - `non_utf8_name_is_other` (`cfg(target_os = "linux")`): a file named with bytes `b"bad\xff"`.
  - `open_file_on_dir_is_is_a_directory`, `list_on_file_is_not_a_directory`, `missing_is_not_found`.
  - `unreadable_dir_is_permission_denied` (unix, skipped when root): mode 0o000 → `PermissionDenied`.
  - `walk_yields_dirs_then_files_and_skips_escapes`: a tree containing an empty dir, a nested file and an escaping symlink → exact `WalkItem` sequence, with no escape.
  - `space_reports_nonzero_total`
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): confined listing, stat, open and walk`.

### Task 4: Browse API

**Files:**
- Create: `crates/server/src/api/mod.rs`, `crates/server/src/api/browse.rs`, `crates/server/tests/browse_api.rs`
- Modify: `crates/shared/src/api.rs`, `crates/server/src/app.rs` (mount `api::routes()`; `/api/me` moves there unchanged), `crates/server/src/error.rs` (`impl From<StorageError>`, `impl From<InvalidPath>`, `ApiError::volume_not_found(name)`)

**Interfaces:**
- Consumes: `Volumes`, `StorageError`, `RelPath`, `CurrentUser`, `authz::Policy` with `Action::Read`.
- Produces:
  ```rust
  // shared::api
  #[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)] #[serde(rename_all = "snake_case")] pub enum EntryKind { Dir, File, Other }
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct Entry { pub name: String, pub kind: EntryKind, pub size: Option<u64>, pub modified_ms: Option<i64> }
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct Listing { pub volume: String, pub path: String, pub entries: Vec<Entry> }
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct Space { pub total: u64, pub available: u64 }
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct VolumeInfo { pub name: String, pub read_only: bool, pub space: Option<Space> }
  ```
  - `GET /api/volumes` → `Vec<VolumeInfo>`. `space` is `None`, with a `warn` log, when statvfs fails.
  - `GET /api/volumes/{volume}/entries?path=<rel>` → `Listing`. A missing `path` means root. `Listing.path` is the normalised path.
  - Every handler extracts `CurrentUser` and calls `Policy.check(&user, volume, Action::Read)` before touching storage.
  - Error mapping (status, code, and message pattern with `{volume}/{path}`, or just `{volume}` for root):
    - `NotFound` → 404 `path_not_found`, `Not found: {volume}/{path}`;
    - `NotADirectory` → 400 `not_a_directory`, `Not a folder: {volume}/{path}`;
    - `IsADirectory` → 400 `is_a_directory`, `Is a folder: {volume}/{path}`;
    - `PermissionDenied` → 403 `permission_denied`, `Permission denied on {volume}/{path}`;
    - `OutsideVolume` → 403 `outside_volume`, `{volume}/{path} points outside the volume`, logged at `warn`;
    - `Io` → `internal`, with the OS error logged and never sent to the client;
    - `InvalidPath` → 400 `invalid_path`;
    - unknown volume → 404 `volume_not_found`, `Volume not found: {volume}`.

- [ ] **Step 1: Write failing tests** in `tests/browse_api.rs`, signed in through the fake provider with the existing cookie-jar helper (move it to `tests/common` if it lives in `auth_flow.rs`):
  - `volumes_require_login` → 401
  - `lists_volumes_with_space`
  - `lists_root_and_subfolder`
  - `unknown_volume_is_volume_not_found`
  - `missing_path_is_path_not_found_with_details`: `details == {"volume":"media","path":"nope"}`
  - `file_as_folder_is_not_a_directory`
  - `traversal_attempts_are_invalid_path`: the raw query strings `path=..`, `path=..%2Fx`, `path=%2e%2e/x`, `path=a/../../x`, `path=%2Fetc`, `path=a%00b` each give 400 `invalid_path`. `GET /api/volumes/..%2F/entries` gives `volume_not_found` or `invalid_path`, never 200 (Review Focus 1).
  - `special_characters_round_trip`: a folder `é #?%` queried with a percent-encoded path lists its file (Review Focus 3).
  - `escaping_symlink_is_outside_volume` → 403 `outside_volume`
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): volumes and listing api`.

### Task 5: File download with Range

**Files:**
- Create: `crates/server/src/files/mod.rs`, `crates/server/src/files/download.rs`, `crates/server/tests/download.rs`
- Modify: `crates/server/src/app.rs` (mount `files::routes()` under `/files`), Cargo manifests (`http-range-header`, `tokio-util` `io`, `percent-encoding`, `httpdate`)

**Interfaces:**
- Consumes: `Volume::open_file`, error mapping (Task 4), `CurrentUser`, `Policy`.
- Produces:
  ```rust
  pub fn routes() -> Router<AppState>; // GET+HEAD /download, GET /zip (Task 6)
  pub fn content_disposition(file_name: &str) -> HeaderValue; // attachment; filename="<ascii fallback>"; filename*=UTF-8''<pct>
  pub fn attachment_headers(headers: &mut HeaderMap, file_name: &str); // the four headers from Global Constraints
  ```
  - `GET /files/download?volume=<v>&path=<rel>` streams the file with `tokio_util::io::ReaderStream` over a `tokio::fs::File::from_std` handle, never reading it whole.
  - Headers: `Accept-Ranges: bytes`, `Content-Length`, `Last-Modified`, and a weak `ETag` `W/"<size>-<mtime_ns hex>"`.
  - Ranges:
    - a single range (`a-b`, `a-`, `-n`) → 206 with `Content-Range`;
    - a multi-range → the full 200;
    - an unsatisfiable range → 416 `range_not_satisfiable` with `Content-Range: bytes */<size>`;
    - an `If-Range` that matches neither the ETag nor `Last-Modified` → the full 200.
  - HEAD returns the same headers and no body.
  - The ASCII fallback of `content_disposition` replaces non-ASCII characters, `"` and `\` with `_`.
  - Logged at `info`: `download` with `user`, `volume`, `path`, `range`.

- [ ] **Step 1: Write failing tests** in `tests/download.rs`:
  - `downloads_whole_file_as_attachment`: 200; the body is equal to the file; `content-type: application/octet-stream`; `content-security-policy: sandbox`; `content-disposition` starts with `attachment`.
  - `html_file_is_never_inline`: `x.html` → the same headers.
  - `range_from_offset_is_partial`: `bytes=5-` → 206, `content-range: bytes 5-<last>/<size>` and the right body.
  - `suffix_and_bounded_ranges`: `bytes=-3` and `bytes=2-4`.
  - `unsatisfiable_range_is_416`
  - `if_range_mismatch_sends_full_file`: an old ETag → 200 with the full body (Review Focus 4).
  - `head_has_headers_and_no_body`
  - `folder_is_is_a_directory`, `traversal_is_invalid_path`, `escaping_symlink_is_outside_volume` (Review Focus 1, 2)
  - `download_requires_login` → 401
  - unit test `content_disposition_encodes_unicode`: `"Été 😀.mkv"` → `attachment; filename="_t_ _.mkv"; filename*=UTF-8''%C3%89t%C3%A9%20%F0%9F%98%80.mkv`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): resumable file download`.

### Task 6: Folder download as a streamed zip

**Files:**
- Create: `crates/server/src/files/zip.rs`, `crates/server/tests/zip.rs`
- Modify: `crates/server/src/files/mod.rs`, Cargo manifests (`zip` with default features off, only what stored entries need)

**Interfaces:**
- Consumes: `Volume::walk`, `Volume::open_file`, `attachment_headers` (Task 5).
- Produces: `GET /files/zip?volume=<v>&path=<rel>` (empty path means the whole volume).
  - **Before the response.** `walk` runs first, so a missing folder, an unreadable folder or a traversal attempt returns the normal JSON error before any byte is sent.
  - **Streaming.** A `spawn_blocking` task writes the archive with `zip::ZipWriter::new_stream` into a `std::io::Write` adapter that sends 64 KiB chunks through a bounded `tokio::sync::mpsc` channel (capacity 8); the response body is the receiving end.
  - **Entries.**
    - Every entry is `CompressionMethod::Stored`, with `large_file(true)` so files over 4 GiB work, and keeps its mtime.
    - Names are UTF-8 and relative to the zipped folder.
    - A directory entry is written for every directory, so empty folders survive.
  - **Archive name.** `<folder name>.zip`, or `<volume>.zip` for the root.
  - **Failures.**
    - An error after headers are sent (a file unreadable mid-stream, or the client gone) stops the writer.
    - The body ends with an error, so the browser marks the download failed instead of keeping a truncated archive.
    - The failure is logged at `warn` with the path.
  - **Logging.** `zip` at `info` with `user`, `volume`, `path`, `files`, `bytes`.

- [ ] **Step 1: Write failing tests** in `tests/zip.rs`:
  - `zips_folder_tree`: read the body back with `zip::ZipArchive` over a `Cursor`. The entry names, contents, empty dir and stored method are all exactly as expected.
  - `zips_volume_root_named_after_volume`: `content-disposition` contains `media.zip`.
  - `escaping_symlink_is_not_followed`: the archive contains no entry for it and no secret bytes (Review Focus 2).
  - `missing_folder_is_json_error_before_streaming` → 404 `path_not_found`.
  - `unreadable_subfolder_is_permission_denied` (unix, skipped when root) → 403 JSON naming the subfolder.
  - `zip_requires_login`, `traversal_is_invalid_path`.
  - `#[ignore]` `zips_file_over_4gib`: a sparse 4.5 GiB file. `ZipArchive` reads it back, with its size and CRC. Run once manually in Step 4.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS, then `cargo test -p explorer-server --release --test zip -- --ignored` → PASS. Record the run time in the report.
- [ ] **Step 5:** Commit `feat(server): streamed zip download of folders`.

### Task 7: Web layout: routes, sidebar, breadcrumb, errors

**Files:**
- Create: `crates/web/src/format.rs`, `crates/web/src/components/sidebar.rs`, `crates/web/src/components/breadcrumb.rs`, `crates/web/src/components/error_panel.rs`, `crates/web/src/components/icons.rs`, `crates/web/src/pages/browse.rs`, `crates/web/src/pages/not_found.rs`
- Modify: `crates/web/src/routes.rs`, `crates/web/src/api.rs`, `crates/web/src/components/shell.rs`, `crates/web/src/pages/home.rs` (becomes the `/` redirect), `crates/web/locales/*.ftl`, `crates/web/Cargo.toml`

**Interfaces:**
- Consumes: `/api/me`, `/api/volumes`, `/api/volumes/{v}/entries`, the shared DTOs and error codes (Task 4).
- Produces:
  ```rust
  // routes.rs
  #[route("/")] Home {}  // fetches volumes, replaces history with Browse { first volume, [] }, or shows the "no volumes" panel
  #[route("/browse/:volume/:..path")] Browse { volume: String, path: Vec<String> }
  #[route("/:..segments")] NotFound { segments: Vec<String> }
  // api.rs
  pub enum ApiFailure { Network, Http(u16), Server(ErrorBody) } // Http when the error body isn't an ErrorBody
  pub fn entries_url(volume: &str, path: &str) -> String;   // /api/volumes/<pct volume>/entries?path=<pct path>
  pub fn download_url(volume: &str, path: &str) -> String;  // /files/download?volume=..&path=..
  pub fn zip_url(volume: &str, path: &str) -> String;       // /files/zip?volume=..&path=..
  // format.rs (host-testable)
  pub fn format_size(bytes: u64, locale: &LanguageIdentifier) -> String;
  ```
  - **Shell.** It keeps the header from M2: app name and avatar menu. Below `md`, a ☰ button in the header opens the sidebar as a drawer; the drawer closes on Escape, on an outside click and on navigation.
  - **Sidebar.**
    - A "Volumes" title, then one link per volume. The current volume is highlighted with the mockup's indigo style.
    - A read-only volume shows a lock icon, with a "Read-only" `title`.
    - The bottom shows `<volume> · <available> free` and a bar for used space, with `role="meter"` and the `aria-value*` attributes.
  - **Breadcrumb.** `volume › seg › … › last`. Every part except the last is a `Link`; the last is bold and carries `aria-current="page"`.
  - **ErrorPanel.**
    - The message comes from the `.ftl` key `error-<code>` with `$path` taken from `details`, falling back to the server message, then `error-unknown`.
    - The request id is shown as `Reference: <id>`, and a "Try again" button refetches.
    - `path_not_found` and `not_a_directory` add a link to the volume root; `volume_not_found` adds a link to `/`.
  - **Text.** Icons are inline SVG components, with no emoji in the UI chrome. All new strings go in en/fr/es.

- [ ] **Step 1: Write failing tests** (host):
  - `format_size_by_locale`:
    - en: 0 → `0 B`, 999 → `999 B`, 1000 → `1 kB`, 82 000 → `82 kB`, 9 800 000 000 → `9.8 GB`, 1 200 000 000 000 → `1.2 TB`;
    - fr: 9 800 000 000 → `9,8 Go`, 532 000 000 → `532 Mo`;
    - es: 1 200 000 → `1,2 MB`.
  - `urls_percent_encode`: `entries_url("media", "é #?%/x")` == `/api/volumes/media/entries?path=%C3%A9%20%23%3F%25%2Fx`. Same encoding for `download_url` and `zip_url`.
  - `browse_route_round_trips`: `Route::Browse { volume: "media", path: ["é #?%", "x"] }.to_string()` parses back to the same value, and `"/browse/media"` parses with an empty `path` (Review Focus 3). If Dioxus doesn't percent-encode segments, `Display` and parsing must be made symmetric through a wrapper segment type; record which in the report.
  - Locale parity test keeps passing.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS, wasm clippy clean.
- [ ] **Step 5:** Commit `feat(web): browse layout with volumes, free space, breadcrumb and error panel`.

### Task 8: Web file table: sorting, virtual list, downloads

**Files:**
- Create: `crates/web/src/listing.rs`, `crates/web/src/components/file_table.rs`, `crates/web/src/components/virtual_list.rs`
- Modify: `crates/web/src/pages/browse.rs`, `crates/web/src/format.rs`, `crates/web/locales/*.ftl`, `crates/web/Cargo.toml` (`js-sys`, web-sys features if needed)

**Interfaces:**
- Consumes: `Listing`, `Entry`, `download_url`, `zip_url`, `format_size` (Task 7).
- Produces:
  ```rust
  // listing.rs (host-testable)
  #[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum SortKey { Name, Size, Modified }
  #[derive(Clone, Copy, PartialEq, Eq, Debug)] pub struct Sort { pub key: SortKey, pub ascending: bool }
  pub fn natural_cmp(a: &str, b: &str) -> Ordering; // case-insensitive, digit runs compared numerically
  pub fn sorted(entries: &[Entry], sort: Sort) -> Vec<Entry>; // dirs first, then files and others; ties by natural_cmp name
  pub fn visible_range(scroll_top: f64, viewport: f64, row_height: f64, total: usize, overscan: usize) -> Range<usize>;
  // format.rs
  pub fn format_modified(ms: i64, now_ms: i64, locale: &LanguageIdentifier) -> String; // wasm: Intl; host: ISO date
  ```
  - **Table.** Columns are Name / Size / Modified (sizes in the Name cell under `md`, as on the mockup's phone view).
    - Header buttons toggle the sort and set `aria-sort`. The default is Name ascending.
    - The sort lives in an app-level signal, so it persists while navigating.
  - **Rows.** Rows are 44 px high, with the icon chosen by kind and extension.
    - Folders: the name is a `Link` to the child `Browse` route.
    - Files: the name is an `a { href: download_url, download: "" }`.
    - Every dir and file row has a trailing download icon `a` (zip for folders), always visible on touch and visible on hover/focus on desktop, with a translated `aria-label`.
    - `Other` rows are greyed and not interactive, with a `title` explaining that they can't be opened.
  - **Modified column.** It shows the time today, "yesterday" from the `.ftl` file, `12 Oct` this year, and `12 Oct 2024` otherwise. Formatting uses `js_sys::Intl::DateTimeFormat` with the active locale.
  - **Virtual list.** A scroll container with `onscroll` and `onresize` renders only `visible_range(.., overscan: 10)`. Top and bottom spacers are sized through Dioxus style properties.
  - **Empty states.** An empty folder shows the empty-folder message; the table header stays.

- [ ] **Step 1: Write failing tests** (host):
  - `natural_cmp_orders_numbers`: `file2 < file10`, `a < B < c`, `Été` sorts after `Ete` deterministically.
  - `sorted_puts_dirs_first_in_both_directions`: Name descending still lists dirs first.
  - `sorted_by_size_and_modified`: `None` values sort last.
  - `visible_range_windows`:
    - (0, 440, 44, 20 000, 10) → `0..20`;
    - (44 000, 440, 44, 20 000, 10) → `990..1020`;
    - near the end it clamps to `total`;
    - total 0 → `0..0` (Review Focus 5).
  - `format_modified_host_fallback`: deterministic ISO output.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS, wasm clippy clean.
- [ ] **Step 5:** Commit `feat(web): sortable virtualised file table with downloads`.

### Task 9: Docs and end-to-end check

**Files:**
- Modify: `docs/architecture.md` (storage module, routes table), `docs/security.md` (path confinement with cap-std, symlink policy, download headers), `docs/installation.md` (mounting volumes under `/data`, read-only mounts with `:ro`, `user:` and ownership, what the startup log says), `docs/configuration.md` (if anything changed), `README.md` (status: browsing and downloading work; screenshot placeholder removed only if a real screenshot of synthetic data is added)

- [ ] **Step 1:** Write the docs. Every example uses `example.com`, `media`, `documents` and synthetic file names.
- [ ] **Step 2: End-to-end check by the controller.**
  - Run the official PocketID image locally from a scratch directory outside the repo (never committed), with a synthetic data dir: nested folders, 20 000 small files in one folder, unicode names, an escaping symlink, a 0o555 volume.
  - Drive headless Chrome through sign-in. Use the WebAuthn virtual authenticator or a PocketID one-time access code; if neither can be automated, say so and leave it to the owner's acceptance.
  - Check the browse flow, the sort, the breadcrumb and the mobile drawer at 390 px.
  - Check that the file download, the resumed download (`curl -r`) and the zip open correctly.
  - Check that a 20 000-entry folder renders with fewer than 100 rows in the DOM, and that there are zero CSP violations.
  - Record the results in the report, and save screenshots to the scratchpad only.
- [ ] **Step 3:** `cargo xtask ci` (with Docker) → PASS.
- [ ] **Step 4:** Commit `docs: volumes, browsing and downloads`.

---

## GitHub setup (once, before Task 1's PR)

Create one issue per task in milestone `M3 Browse & download`, linking to this plan. Labels:
- Task 1: `backend`
- Task 2: `backend,security`
- Task 3: `backend,security`
- Task 4: `backend`
- Task 5: `backend,security`
- Task 6: `backend`
- Task 7: `frontend`
- Task 8: `frontend`
- Task 9: `docs`

Each PR body says `Closes #<issue>`.

## Manual acceptance by the owner (after merge)

On the NAS:
- Browse each volume and check the free space against `df -H`.
- Download a large file, interrupt it and resume it.
- Download a folder as a zip and open it.
- Check the mobile layout on a phone.
