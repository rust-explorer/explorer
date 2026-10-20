# Architecture

Explorer is one Docker container: an Axum server that serves a JSON API and a Dioxus web
app compiled to WebAssembly. A reverse proxy terminates TLS in front of it.

```
browser ──HTTPS──> reverse proxy ──HTTP──> explorer container
                                            ├─ Axum server (:8080)
                                            │   ├─ /healthz    health check
                                            │   ├─ /api/*      JSON API
                                            │   ├─ /auth/*     OIDC login/callback/logout
                                            │   ├─ /files/*    upload (tus) / download / zip
                                            │   └─ /*          web bundle (WASM), SPA fallback
                                            ├─ /data/<volume>  mounted volumes
                                            └─ /config         state (SQLite, thumbnail cache)
```

Status: `/healthz`, the web bundle, sign-in (`/auth/*`, `/api/me`), browsing
(`/api/volumes*`), downloads (`/files/download`, `/files/zip`) and resumable uploads
(`/files/uploads*`, `/api/uploads`, `/api/volumes/{volume}/folders`) are implemented; file
operations (rename, move, delete and the like) are planned.

## Crates

| Crate | Path | Role |
|---|---|---|
| `explorer-server` | `crates/server` | Axum server, binary `explorer`. |
| `explorer-web` | `crates/web` | Dioxus web app (WASM), styled with Tailwind, localised (`en`, `fr`, `es`). |
| `explorer-shared` | `crates/shared` | API types (DTOs, error body) used by both sides. |

The web app talks to the server over a plain REST-style JSON API rather than Dioxus
server functions, so the API stays usable by other clients.

## Request flow

1. Every request gets an `X-Request-Id` (kept if the client sent one), propagated to the
   response and recorded on the request's tracing span.
2. Routing: `/healthz`, then `/api/*`, `/auth/*` and `/files/*` (JSON errors, never HTML),
   then the static bundle. Unknown non-asset paths fall back to `index.html` for client-side
   routing; missing files under `/assets/` are real 404s.
3. Errors use one JSON shape: `code`, `message`, optional `details`, and the request ID.
4. Security headers (CSP suited to a WASM app, `X-Content-Type-Options`,
   `Referrer-Policy`) are set on every response.

## Authentication

Explorer is a backend-for-frontend: the server is the OIDC client, the browser only holds
an opaque session cookie. See [security.md](security.md) for the guarantees.

| Route | Purpose |
|---|---|
| `GET /auth/login?return_to=` | Starts sign-in: stores state, nonce and PKCE verifier in the session, redirects to the provider. |
| `GET /auth/callback` | Provider redirect (query response mode). Verifies state, exchanges the code, verifies the ID token, checks the group, then redirects to `return_to` or to `/access-denied` or `/signin-error?reason=`. |
| `POST /auth/logout` | Flushes the session; returns JSON `{redirect_to}` (the provider's end-session URL, or `/`). |
| `GET /api/me` | The signed-in user, or 401 `unauthenticated`. |

The web app boots by calling `/api/me`; on 401 it shows a sign-in page that links to
`/auth/login`. `/access-denied` and `/signin-error` are SPA pages.

## Volumes and browsing

A volume is a real directory (not a symlink) directly under `/data`. Volumes are
discovered once at startup and sorted by name; adding or removing a mount requires a
restart. All client paths are a pair `(volume, path)`: `volume` is a volume name and `path`
is relative, `/`-separated, with `""` meaning the volume root. The server never accepts an
absolute or host path. See [security.md](security.md) for how paths are confined.

| Route | Purpose |
|---|---|
| `GET /api/volumes` | The volumes: name, `read_only`, and free space (`space`, `null` when it cannot be read). |
| `GET /api/volumes/{volume}/entries?path=` | The entries of a folder, sorted by name: `name`, `kind` (`dir`, `file` or `other`), `size` (files only), `modified_ms`. |
| `GET`, `HEAD /files/download?volume=&path=` | One file, as an attachment. Supports a single `Range`, a strong `ETag` and `If-Range`. |
| `GET /files/zip?volume=&path=` | A folder as a zip, streamed. `path` is required: pass it empty for the whole volume. |

All of them require a signed-in user. Failures use the JSON error shape with these codes
(`details` carries `volume` and `path` when they concern a path):

| Code | Status | Meaning |
|---|---|---|
| `volume_not_found` | 404 | No such volume. |
| `path_not_found` | 404 | The path does not exist. |
| `invalid_path` | 400 | The path is malformed (see the rules in [security.md](security.md)), or a required parameter is missing. |
| `not_a_directory` | 400 | A listing or zip of something that is not a folder. |
| `is_a_directory` | 400 | A download of a folder; use `/files/zip`. |
| `permission_denied` | 403 | The container user cannot read it. |
| `outside_volume` | 403 | The path resolves outside the volume (symlink escape). |
| `range_not_satisfiable` | 416 | The requested range cannot be satisfied (it starts past the end of the file, the file is empty, or the range is `bytes=-0`). |

Any other I/O failure is a 500 `internal` whose cause is only in the server log.

Downloads: the body is streamed from the open file, so memory use does not depend on the
file size. The zip is streamed without compression (`stored`) with zip64 entries, so
there is no size limit and no temporary file. The folder is walked before the first byte is
sent, so a missing or unreadable folder is a normal JSON error. A failure after streaming
has started (for example a file that becomes unreadable) cannot change the status code:
the server aborts the body, the browser reports a failed download, and the reason is in
the server log (`zip aborted`, with the path). A client that stops reading for 60 seconds
is treated as gone and the zip is aborted the same way.

## Uploads

Uploads follow the [tus](https://tus.io/protocols/resumable-upload) 1.0.0 protocol with the
`creation` and `termination` extensions, so a transfer survives a dropped connection, a
reload or a laptop going to sleep. The server never holds a whole file in memory.

| Route | Purpose |
|---|---|
| `OPTIONS /files/uploads` | Discovery: `Tus-Version` and `Tus-Extension: creation,termination`. |
| `POST /files/uploads` | Creates an upload. `Upload-Length` is required (no deferred length, no body). `Upload-Metadata` carries `volume` and `path` (the destination file path, including the file name), and optionally `modified` (the file's `lastModified`, integer milliseconds) and `conflict` (`fail`, the default, `replace` or `keep_both`); values are base64, unknown keys are ignored. Answers 201 with `Location: /files/uploads/{id}` and `Upload-Offset: 0`. |
| `HEAD /files/uploads/{id}` | The current `Upload-Offset` and `Upload-Length`, with `Cache-Control: no-store`. |
| `PATCH /files/uploads/{id}` | Appends the body at `Upload-Offset` (`Content-Type: application/offset+octet-stream`). Answers 204 with the new offset. |
| `DELETE /files/uploads/{id}` | Cancels the upload and removes its staged data. |
| `GET /api/uploads` | The signed-in user's unfinished uploads (`id`, `volume`, `path`, `length`, `offset`, `modified_ms`), so a reloaded page can resume them. |
| `POST /api/volumes/{volume}/folders` | Body `{"path": "..."}`. Creates a folder and its missing parents; an existing folder is fine. Answers 204. Used to upload folder trees, including empty folders. |

`Tus-Resumable: 1.0.0` is required on every request under `/files/uploads` except
`OPTIONS`; otherwise the answer is 412 `unsupported_tus_version` with `Tus-Version`. Every
response from that prefix carries `Tus-Resumable`, errors and unknown paths included (a
layer in front of the CSRF check adds it, so a rejected cross-origin request carries it
too). All routes require a signed-in user, and the upload and folder routes also require
write access to the volume. An upload belongs to the `sub` that created it: another user's
id answers `upload_not_found`.

New error codes (JSON error shape as above):

| Code | Status | Meaning |
|---|---|---|
| `already_exists` | 409 | The destination exists and `conflict` is `fail` (or no free "keep both" name was found up to `(999)`). |
| `read_only_volume` | 403 | The volume is read-only. |
| `insufficient_storage` | 507 | `Upload-Length` is larger than the volume's free space, or the disk filled up while writing. |
| `upload_not_found` | 404 | Unknown id, another user's upload, or an upload whose staged file is gone. |
| `offset_mismatch` | 409 | `Upload-Offset` is not the server's offset; `details.offset` carries the right one. |
| `upload_busy` | 423 | Another request is writing or finishing this upload. |
| `upload_too_large` | 413 | The body is longer than `Upload-Length`. |
| `invalid_upload` | 400 | Missing or malformed `Upload-Length` or `Upload-Metadata`. |
| `unsupported_tus_version` | 412 | `Tus-Resumable` missing or not `1.0.0`. |
| `unsupported_media_type` | 415 | `PATCH` without `application/offset+octet-stream`. |
| `cross_device` | 500 | The destination folder is a separate mount nested in the volume, so the staged file cannot be renamed into it. |

An empty destination path (the volume root) is `invalid_path`; `is_a_directory`,
`not_a_directory` and `permission_denied` from the table above also apply to destinations.

How an upload is stored:

1. Creation checks the destination (a parent that is a file, an existing file with
   `conflict=fail`, a folder with `replace`) and the free space, creates the staged file and
   inserts a record in the `uploads` table of `explorer.db` (id, owner `sub`, volume, path,
   length, `modified`, conflict policy, timestamps).
2. Data is written to `.explorer-uploads/<id>` at the root of the volume. The directory is
   created on first upload with mode `0700`, the file with `0666` less the umask, both owned
   by the container user; the final file keeps that mode, like any file the container
   creates. The offset reported to the client is the staged file's length, so it is right
   after a crash. `.explorer-uploads` is rejected as the first component of any client path
   (`invalid_path`, reason `reserved name`). It is also recognised by device and inode, so
   it is neither listed, zipped, read nor written through a symlink to the volume root or
   to the directory itself (`path_not_found`).
3. Each `PATCH` runs as its own task holding an in-memory per-upload lock, so a second
   writer gets `upload_busy`, and a client that disconnects does not leave a half-recorded
   write. A body that sends nothing for 60 seconds is treated as a disconnect; what arrived
   is kept and flushed to disk.
4. When the staged file reaches `Upload-Length`, the server creates the missing parent
   folders, applies `modified` as the file's modification time, syncs it and renames it
   into place. `fail` and `keep_both` use a no-replace rename, so a file that appears
   during the upload is never overwritten (with `fail` the upload then takes a "keep both"
   name and the log says so); `replace` renames over the destination but never over a
   folder. "Keep both" names `Film.mkv` as `Film (1).mkv`, then `(2)` and so on: the suffix
   goes before the last extension, and a name whose only dot is the first character has none
   (`.bashrc (1)`). The record then gets its `final_path`; a retried empty `PATCH` at the end
   is answered 204, which makes the last chunk safe to resend. If the commit fails, the
   staged file stays complete and `HEAD` reports the full length; an empty `PATCH` at the end
   retries the commit, and the client sends one before it shows the upload as done. A
   zero-length upload is committed by the creation request itself.
5. The rename stays on one filesystem. A folder that is a separate mount nested inside a
   volume cannot receive uploads: the commit fails with 500 `cross_device`, which the client
   shows as failed instead of retrying, and the log says that the destination is on another
   filesystem than the volume. Mount such a folder as its own volume.

Abandoned uploads: a record not touched for 24 hours is deleted with its staged file, and
so is a staged file older than that without a record. The sweep runs at startup and then
every hour; it logs each abandoned upload it removes at `info` (`action` = `sweep`), then
a summary of what it removed.

Logs (`info`, with `user` = the OIDC `sub`, `action`, `volume`, `path`): upload started
(with `length`), completed (with `bytes` and `final_path`), cancelled, folder created, and
each sweep that removed something. Failures log at `warn`. File contents and metadata
values other than these fields are never logged.

## Web app

Routes: `/` (opens the first volume, or says there is none), `/browse/<volume>/<path>`,
`/access-denied`, `/signin-error` and a not-found page. The folder lives in the URL, so
links, reload and the back button work. The sidebar lists the volumes with their free
space and a read-only marker. The folder is shown as a table sorted by name, size or date
(folders first), rendered through a virtualised list so a folder with tens of thousands of
entries keeps the DOM small. Sizes use SI units (1 kB = 1000 B). Each file row links to
`/files/download` and each folder row to `/files/zip`; these are plain links, so the
browser handles the download and shows any failure in its own download list.

Uploads: the header has an Upload button (a floating "+" on narrow screens) that offers
files or a folder, and files or folders can be dropped on the folder listing. On a read-only volume
the controls are disabled. A panel lists the transfers with their progress, a cancel button
per item, and how many dropped or picked items could not be read and were skipped; retries
are automatic. Files dropped anywhere else in the page are ignored rather than opened by the
browser. The engine lives in `crates/web/src/uploads`:

- A file is cut into 16 MiB chunks, each sent as one `PATCH`; at most 3 files upload at
  once. Before creating an upload, the client resumes a pending one (from `GET /api/uploads`)
  whose volume, path, size and modification time all match, so reloading and picking the
  same file again continues from the server's offset.
- After a transient failure (network error, 5xx other than 507, `upload_busy`,
  `offset_mismatch`) the item asks the server for its offset and retries after
  `min(1 s x 2^(attempt-1), 30 s)`, until the user cancels. A `PATCH` that reports no
  progress for 120 seconds is aborted and counts as a network error. An upload the server no
  longer knows (`upload_not_found`) is recreated from the start, once. When that offset is
  the full length, an empty `PATCH` confirms the commit before the item is marked done.
- An `already_exists` answer opens a dialog: Replace, Keep both or Cancel, with an "apply to
  all" option for the rest of the batch. Folders that already exist are merged into, never a
  conflict.
- Folder trees are sent as their files plus a `POST .../folders` for each empty folder. The
  folder picker only exposes files, so it cannot see empty folders; drag and drop reads the
  directory entries and does create them.
- The sidebar's free space and the listing refresh while uploads run (at most every 2 s, and
  within 2 s of any completion) and when they end. The browser asks for confirmation before the page is closed with
  transfers active.
- Everything is same-origin `XHR`/`fetch`, so the CSP's `connect-src 'self'` is enough.

## Server modules

Implemented:

- `config` — loads and validates the environment at startup, reporting every problem.
- `app` — router, middleware stack, shared state.
- `web` — static bundle serving, cache headers, security headers.
- `error` — the JSON error type.
- `telemetry` — `tracing` setup (pretty or JSON).
- `healthcheck` — client used by `explorer healthcheck` for the Docker healthcheck.
- `oidc` — provider discovery (retried at startup), authorization URL, code exchange, ID
  token verification, groups, end-session URL.
- `auth` — `/auth/*` handlers and the `CurrentUser` extractor.
- `session` — cookie and SQLite-backed session layer.
- `db` — SQLite pool for `/config/explorer.db` (WAL), with a writability check.
- `csrf` — `Origin`/`Referer` check on state-changing requests.
- `authz` — authorization policy; handlers pass the user, the volume and the action, so
  finer rules can be added later.
- `storage` — all filesystem access; handlers never touch the filesystem directly.
  - `path` — `RelPath`, the validated relative path type.
  - `volumes` — discovery under `/data` and read-only detection.
  - `ops` — listing, `stat`, opening files, walking a folder, free space. Blocking calls
    on [cap-std](https://docs.rs/cap-std) directories, run on Tokio's blocking pool.
  - `staging` and `write` — the per-volume staging directory and its files, the upload
    target checks, folder creation and the final rename.
  - `error` — storage errors, mapped to API error codes in `error`.
- `api` — `GET /api/volumes`, `GET /api/volumes/{volume}/entries`, `POST /api/volumes/{volume}/folders` and `GET /api/uploads`.
- `access` — handler helpers: volume lookup, policy check, read-only rejection and path
  parsing in one call, and running a blocking storage call on a volume.
- `files` — `/files/download`, `/files/zip` and the `/files/uploads` mount.
- `uploads` — the tus handlers (`tus`), the `uploads` table and in-memory locks (`store`),
  and the abandoned-upload sweep (`cleanup`).
