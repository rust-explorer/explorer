# Architecture

Explorer is one Docker container: an Axum server that serves a JSON API and a Dioxus web
app compiled to WebAssembly. A reverse proxy terminates TLS in front of it.

```
browser ──HTTPS──> reverse proxy ──HTTP──> explorer container
                                            ├─ Axum server (:8080)
                                            │   ├─ /healthz    health check
                                            │   ├─ /api/*      JSON API
                                            │   ├─ /auth/*     OIDC login/callback/logout
                                            │   ├─ /files/*    upload (tus) / download / zip / thumbnail
                                            │   └─ /*          web bundle (WASM), SPA fallback
                                            ├─ /data/<volume>  mounted volumes
                                            └─ /config         state (SQLite, thumbnail cache)
```

Status: `/healthz`, the web bundle, sign-in (`/auth/*`, `/api/me`), browsing
(`/api/volumes*`), downloads (`/files/download`, `/files/zip`) and resumable uploads
(`/files/uploads*`, `/api/uploads`, `/api/volumes/{volume}/folders`) are implemented, and so
are managing files: new folder, rename, multi-select and zip of several entries, and delete,
copy and move as background tasks (`/api/volumes/{volume}/rename`, `/api/tasks*`). The polish
milestone adds a refresh button, favorite folders, search by name, a grid view with image
thumbnails, space usage, dark mode and a read-only notice.

## Crates

| Crate | Path | Role |
|---|---|---|
| `explorer-server` | `crates/server` | Axum server, binary `explorer`. |
| `explorer-web` | `crates/web` | Dioxus web app (WASM), styled with Tailwind, localised (`en`, `fr`, `es`). |
| `explorer-shared` | `crates/shared` | Types used by both sides: the API DTOs, one module per domain under `api/` (`auth`, `browse`, `conflict`, `error`, `favorites`, `health`, `search`, `tasks`, `uploads`, `usage`; `conflict` holds the conflict policy that uploads, renames, copies and moves share), and the tus metadata encoding. |

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
| `GET /files/zip?volume=&path=` | A folder as a zip, streamed. `path` is required: pass it empty for the whole volume. Repeat `name=` to archive only those entries of the folder (files or folders, each one a single name, as stored in the zip under that name; a repeated name counts once). Entries carry their modification time, in UTC in the DOS field and exactly in an extended timestamp. With `selection=<id>` in place of the other parameters, the entries of a stored selection (below). |
| `POST /files/zip/selections` | Stores a selection too large for a URL: `{volume, path, names}` (each name a single entry of the folder, at least one), answered 201 `{id}`. `GET /files/zip?selection=<id>` then zips it, for the same user, during 10 minutes; a user keeps their 16 newest selections. The body is limited to 2 MB. Nothing is read from disk until the zip, which checks everything again. |

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
| `too_many_zips` | 429 | 4 zips are already streaming on the server, or 2 for this user; try again when one ends. The web app starts zips with `<a download>`, which cannot read this answer: the browser shows a failed download (often "server problem") and the app shows nothing. |
| `selection_expired` | 404 | No stored selection with this id for this user: it expired, was replaced by newer ones, or never existed. |
| `range_not_satisfiable` | 416 | The requested range cannot be satisfied (it starts past the end of the file, the file is empty, or the range is `bytes=-0`). |

Any other I/O failure is a 500 `internal` whose cause is only in the server log.

Downloads: the body is streamed from the open file, so memory use does not depend on the
file size. The zip is streamed without compression (`stored`) with zip64 entries, so
there is no size limit and no temporary file. The folder is checked and listed before the
first byte is sent, so a missing or unreadable folder is a normal JSON error; the rest of
the tree is walked while the archive streams, one folder listing per level in memory, so the
first bytes leave at once and memory does not grow with the tree. At most 4 zips stream at
once across all users, and 2 per user so that one user cannot take every slot
(`too_many_zips` beyond); a zip stops, and frees its slot, as soon as the client goes away
or stops reading for 60 seconds. The web app hands the zip URL to the browser as a
download, so it never sees a refusal: the browser lists the download as failed. Checking for
a free slot first would not help, since it could be taken before the download starts.

Nothing is left out silently. Entries are reached by their names on disk, so a name that is
not UTF-8, holds a control character or lies under a path longer than 4096 bytes is
archived too, under a safe name: invalid UTF-8 becomes `�`, control characters and `\`
become `_`, and a name that then collides with another one in its folder gets ` (2)`. What
cannot be archived (a sub-folder or file the server cannot read, a symlink to a folder
other than one selected itself in a multi-entry download, a
broken or escaping symlink, a FIFO, socket or device) is skipped, and the archive then ends
with `explorer-skipped.txt` at its top (renamed if that name is taken), listing each one
with the reason (the first 10,000, then a count); the server logs `zip left entries out`
at `warn`. A failure after streaming has started that is not one of these (a file that
fails mid-read) cannot change the status code: the server aborts the body, the browser
reports a failed download, and the reason is in the server log (`zip aborted`, with the
path). A client that stops reading for 60 seconds is treated as gone and the zip is aborted
the same way.

## Uploads

Uploads follow the [tus](https://tus.io/protocols/resumable-upload) 1.0.0 protocol with the
`creation` and `termination` extensions, so a transfer survives a dropped connection, a
reload or a laptop going to sleep. The server never holds a whole file in memory.

| Route | Purpose |
|---|---|
| `OPTIONS /files/uploads` | Discovery: `Tus-Version` and `Tus-Extension: creation,termination`. |
| `POST /files/uploads` | Creates an upload. `Upload-Length` is required (no deferred length, no body). `Upload-Metadata` carries `volume` and `path` (the destination file path, including the file name), and optionally `modified` (the file's `lastModified`, integer milliseconds) `conflict` (`fail`, the default, `replace` or `keep_both`) and `fingerprint` (16 lowercase hex digits, see below); values are base64, unknown keys are ignored. Answers 201 with `Location: /files/uploads/{id}` and `Upload-Offset: 0`. |
| `HEAD /files/uploads/{id}` | The current `Upload-Offset` and `Upload-Length`, with `Cache-Control: no-store`. |
| `PATCH /files/uploads/{id}` | Appends the body at `Upload-Offset` (`Content-Type: application/offset+octet-stream`). Answers 204 with the new offset. |
| `DELETE /files/uploads/{id}` | Cancels the upload and removes its staged data. Answers 204, or 409 `upload_committed` when the upload was already complete (the file stays in place). |
| `GET /api/uploads` | The signed-in user's unfinished uploads (`id`, `volume`, `path`, `length`, `offset`, `modified_ms`, `conflict`, `fingerprint`), so a reloaded page can resume them. |
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
| `insufficient_storage` | 507 | `Upload-Length` is larger than the volume's free space less what its unfinished uploads still have to receive, or the disk filled up while writing. |
| `too_many_uploads` | 429 | The user already has 1000 unfinished uploads (`MAX_PENDING_UPLOADS`). |
| `upload_committed` | 409 | `DELETE` of an upload that was already committed; `details.final_path` is where the file is. The record is dropped all the same. |
| `too_many_patches` | 429 | The user already has 8 `PATCH` requests in flight (`MAX_PATCHES_PER_USER`); retry later. |
| `upload_not_found` | 404 | Unknown id, another user's upload, or an upload whose staged file is gone. |
| `offset_mismatch` | 409 | `Upload-Offset` is not the server's offset; `details.offset` carries the right one. |
| `upload_busy` | 423 | Another request is writing or finishing this upload. |
| `upload_too_large` | 413 | The body is longer than `Upload-Length`. |
| `invalid_upload` | 400 | Missing or malformed `Upload-Length` or `Upload-Metadata`. |
| `unsupported_tus_version` | 412 | `Tus-Resumable` missing or not `1.0.0`. |
| `unsupported_media_type` | 415 | `PATCH` without `application/offset+octet-stream`. |
| `cross_device` | 500 | The destination folder is a separate mount nested in the volume, so the staged file cannot be renamed into it. A copy or a move that has to copy can hit it too. |

An empty destination path (the volume root) is `invalid_path`; `is_a_directory`,
`not_a_directory` and `permission_denied` from the table above also apply to destinations.

How an upload is stored:

1. Creation checks the user's count of unfinished uploads, the destination (a parent that
   is a file, an existing file with `conflict=fail`, a folder with `replace`) and the free
   space less the bytes the volume's unfinished uploads still owe, creates the staged file and
   inserts a record in the `uploads` table of `explorer.db` (id, owner `sub`, volume, path,
   length, `modified`, conflict policy, fingerprint, timestamps).
2. Data is written to `.explorer-uploads/<id>` at the root of the volume. The directory is
   created on first upload with mode `0700`, the file with `0666` less the umask, both owned
   by the container user; the final file keeps that mode, like any file the container
   creates. The offset reported to the client is the staged file's length, so it is right
   after a crash. `.explorer-uploads`, in any letter case, is rejected as the first component of any client path
   (`invalid_path`, reason `reserved name`). It is also recognised by device and inode, so
   it is neither listed, zipped, read nor written through a symlink to the volume root or
   to the directory itself (`path_not_found`).
3. Each `PATCH` runs as its own task holding an in-memory per-upload lock, so a second
   writer gets `upload_busy`, and a client that disconnects does not leave a half-recorded
   write. A body that sends nothing for 60 seconds is treated as a disconnect; what arrived
   is kept and flushed to disk.
4. When the staged file reaches `Upload-Length`, the server creates the missing parent
   folders, syncs the file and renames it into place, then applies `modified` as its
   modification time (only once placed, so a staged file never looks old to the sweep). `fail` and `keep_both` use a no-replace rename, so a file that appears
   during the upload is never overwritten (with `fail` the upload then takes a "keep both"
   name and the log says so); `replace` renames over the destination but never over a
   folder. "Keep both" names `Film.mkv` as `Film (1).mkv`, then `(2)` and so on: the suffix
   goes before the last extension, and a name whose only dot is the first character has none
   (`.bashrc (1)`). A name that would then pass 255 bytes is shortened at the end of its
   stem (of the whole name if the extension leaves no room). The record then gets its `final_path`; a retried empty `PATCH` at the end
   is answered 204, which makes the last chunk safe to resend. If the commit fails, the
   staged file stays complete and `HEAD` reports the full length; an empty `PATCH` at the end
   retries the commit, and the client sends one before it shows the upload as done. A
   zero-length upload is committed by the creation request itself.
5. The rename stays on one filesystem. A folder that is a separate mount nested inside a
   volume cannot receive uploads: the commit fails with 500 `cross_device`, which the client
   shows as failed instead of retrying, and the log says that the destination is on another
   filesystem than the volume. Mount such a folder as its own volume.

Abandoned uploads: a record not touched for 24 hours is deleted with its staged file, and
so is a staged file older than that without a record. The sweep checks each record again
once it holds the upload's lock, so an upload resumed meanwhile is kept. The sweep runs at startup and then
every hour; it logs each abandoned upload it removes at `info` (`action` = `sweep`), then
a summary of what it removed.
A creation also removes the same user's unfinished uploads to the same volume and path
that had no `PATCH` for 2 minutes and that no request holds (`upload superseded by a new
upload`, `info`), since the client creates a new upload only when it does not resume them;
otherwise an upload abandoned after a crash would keep its free space reserved for a day.

Logs (`info`, with `user` = the OIDC `sub`, `action`, `volume`, `path`): upload started
(with `length`), completed (with `bytes` and `final_path`), cancelled, folder created, and
each sweep that removed something. Failures log at `warn`. File contents and metadata
values other than these fields are never logged.

## Managing files

Creating, renaming, deleting, copying and moving require a signed-in user and write access
to the volume they change (a copy only needs to read its source). They apply to files and
folders alike.

| Route | Purpose |
|---|---|
| `POST /api/volumes/{volume}/folders` | As above; with `"exclusive": true` it creates just that folder and answers `already_exists` if the name is taken, instead of merging. A missing parent is then `path_not_found`. |
| `POST /api/volumes/{volume}/rename` | Body `{"path", "name", "conflict"}` (`conflict` defaults to `fail`). Renames the entry at `path` to `name` in the same folder and answers `{"path"}`, the new path, which differs from the requested one under `keep_both`. Renaming to the current name is a no-op. The volume root cannot be renamed. |
| `GET /api/tasks` | The signed-in user's tasks, oldest first. |
| `POST /api/tasks` | Queues a task and answers 202 with its state. |
| `DELETE /api/tasks/{id}` | Cancels a queued or running task, or forgets a finished one. Answers 204. |

A task is `{"op": "delete", "source"}`, `{"op": "copy", "source", "dest_dir", "conflict"}` or
`{"op": "move", ...}`, where `source` and `dest_dir` are `{volume, path}` locations and
`dest_dir` is the folder the entry goes into (under its own name). One task handles one
entry; the web app starts one per selected entry. `TaskInfo` carries the `id`, the `spec`,
the `state` (`queued`, `running`, `done`, `failed`, `cancelled`), `files_done` and
`files_total`, `bytes_done` and `bytes_total`, `skipped` (entries a copy leaves out, see
[security.md](security.md#managing-files)), `final_path` (where a copy or move ended up) and,
for a failed task, `error` in the usual JSON error shape.

`POST /api/tasks` checks the task before queuing it, so what can be known to fail at once is a
normal error and no task exists: the source is missing (a delete of a missing entry is
`path_not_found`), the destination is not a folder, the name is taken under `fail`, `replace`
meets a folder (as source or as existing target), or the move or copy is refused. An `already_exists` from
a copy or move carries `details.kind` (`dir` or `file`) for the existing target, so that a
client knows whether it may offer Replace. Whatever fails later is reported by the task as
`failed`, with the request id of the `POST`.

New error codes (JSON error shape as above):

| Code | Status | Meaning |
|---|---|---|
| `invalid_destination` | 400 | A copy or move of a folder into itself or one of its descendants, or a move into the folder the entry is already in. `details` name the entry, on its source volume. |
| `task_not_found` | 404 | Unknown task id, or another user's. |
| `changed_during_move` | 409 | A move to another volume found, when deleting the source, that a file had changed or been replaced since it was copied. That file stays in the source (with the folders holding it) and the copy is in the destination; the rest of the move is done. `details` name the file, on its source volume. |
| `too_many_entries` | 400 | A copy, or a move to another volume, of a folder holding more than 1,000,000 files and folders in all. Nothing was copied. |
| `too_many_tasks` | 429 | The user already has 1000 tasks queued or running. The web app posts the task again once some have finished. |

Existing codes also apply: `already_exists` (409), `is_a_directory` (`replace` over or of a
folder), `not_a_directory`, `invalid_path` (a bad name, the volume root, a reserved path, a
malformed body), `read_only_volume`, `insufficient_storage` and `cross_device`. A failure on
the source side of a copy or move carries the source volume in `details`, one on the
destination side the destination volume.

How tasks run:

- The registry is in memory (`crates/server/src/tasks`): a restart loses every task, and a
  task that was running stops with the process. A task belongs to the `sub` that created it.
  A finished task is listed for one hour unless dismissed, and only a user's 1000 most
  recently created finished tasks are kept: older ones are forgotten first.
- At most 2 tasks run at once, across all users; the others stay `queued`. Each runs on
  Tokio's blocking pool. A queued task is cancelled at once; a running copy, or the copy
  phase of a move, when it next checks (between 1 MiB chunks, and between entries of a
  folder). A running delete cannot be cancelled: it is a single removal and ends `done`, and
  the deleting phase of a move runs to the end too.
- Delete removes a file, a symlink (not its target) or a folder tree. It is not a trash:
  deleted data is gone.
- Copy streams each file through a staged file in the destination volume's staging directory
  and renames it into place, like an upload. A folder is copied under a freshly created
  name, never merged into an existing folder. On failure or cancellation the staged files and
  the partly copied folder are removed. Progress is files and bytes, planned before the copy.
- Move within a volume is one rename (no data is copied). Across volumes, or when the rename
  hits a nested mount, it copies, then deletes each copied file from the source and the
  folders left empty. A failed or cancelled copy leaves the source whole.
- Logs (`info`, with `user`, `action`, `volume`, `path`, and `dest_volume` and `final_path`
  for a transfer): a rename, a delete, a finished transfer (with `files`, `bytes` and
  `skipped` for a copy); failures at `warn`.

## Favorites, search and thumbnails

All of these only read, so they work on read-only volumes.

| Route | Purpose |
|---|---|
| `GET /api/favorites` | The user's favorite folders, oldest first: `volume`, `path` and `exists` (false once the volume or the folder is gone or no longer a folder). |
| `PUT /api/favorites` | Body `{"volume", "path"}`. Adds a folder (it must exist and be readable); adding one that is already there is a no-op. Answers 204. |
| `DELETE /api/favorites?volume=&path=` | Removes one, whether or not it still exists. Answers 204. |
| `GET /api/volumes/{volume}/search?q=` | Case- and accent-insensitive substring match on names under the whole volume, whatever their Unicode normalisation (a name stored decomposed, as macOS clients write it, matches a query typed composed): `{volume, query, hits, truncated}`, each hit with `path`, `kind` (`dir` or `file`), `size` and `modified_ms`. |
| `GET /files/thumbnail?volume=&path=&v=` | A preview of an image, at most 320 px on its longest side. `v` only busts the browser cache. |

Favorites are stored per user (the OIDC `sub`) in the `favorites` table of `explorer.db`, at
most 100 per user; the 101st is 409 `favorites_full`. They are bookmarks: nothing checks them
again until they are listed, and a missing folder is shown greyed out until the user removes
it.

Search walks the volume breadth-first from its root, so shallow matches come first, under the
same rules as a zip (the staging folder is skipped, symlinked folders are not entered,
entries listed as `other` are never matched; an unreadable folder is skipped). The query is
trimmed and needs at least 2 characters (`invalid_path` otherwise). It stops, with
`truncated: true`, at 200 hits or after 20 seconds. The walk runs on the blocking pool and
stops as soon as the client drops the request (a new keystroke, a closed tab), so a
superseded search does not keep a thread busy. Nothing is indexed.

Thumbnails are made for `jpg`, `jpeg`, `png`, `gif` and `webp` files of at most 64 MiB;
anything else, a missing file or an image that cannot be decoded is 404
`thumbnail_unavailable` (a decode failure is logged at `debug` only). The image is decoded
within limits (its estimated decode memory at most 512 MiB, see
[security.md](security.md#thumbnails)), shrunk, rotated according to its EXIF orientation and **re-encoded**: PNG when
it has transparency, JPEG otherwise. The user's bytes are never sent. At most 2 images are
decoded at once across all users; a request that is cancelled mid-render keeps its slot until
the work ends. Results are cached as files under `/config/cache/thumbnails` (see
[configuration.md](configuration.md#thumbnail-cache)), keyed by a hash of the volume, path,
size, modification time and a rendering version, so an edited file gets a new thumbnail. The
response carries a strong `ETag` and `Cache-Control: private, max-age=604800`, and `If-None-Match`
is answered 304. See [security.md](security.md#thumbnails) for the headers.

New error codes (JSON error shape as above):

| Code | Status | Meaning |
|---|---|---|
| `favorites_full` | 409 | The user already has 100 favorites. |
| `thumbnail_unavailable` | 404 | No thumbnail for this file: unsupported type, too large, missing, or not decodable. |

Logs: `favorite_add` and `favorite_remove` at `info`; `search` (with `hits`, `truncated`,
`elapsed_ms`) and `thumbnail` at `debug`. A search query is only logged at `debug`.

## Space usage

A scan is a task like a copy: `POST /api/tasks` with `{"op": "usage", "source": {volume, path}}`
(the source must be a folder the user can read, on any volume, read-only ones included). It
shares the queue of 2 running tasks, can be cancelled while it runs, and reports `files_done`
and `bytes_done` as it goes. When it is `done`, the result is read with
`GET /api/tasks/{id}/usage?path=`: one folder of the scanned tree (the scanned folder by
default; `path` must be at or below it, else 404 `path_not_found`) as `UsageView`: `bytes`,
`files`, `dirs`, `incomplete`, `collapsed`, and its `items` biggest first. An item is a
sub-folder, one of the 20 biggest files of the folder, or `other_files` (the smaller files
together, and anything beyond 200 items). Navigating the tree costs no new scan.

How a scan counts:

- Sizes are the space allocated on disk (`st_blocks` x 512), not the apparent size, so a
  sparse or compressed file counts for what it takes.
- A file with several hard links is counted once, up to 100,000 such files per scan; beyond
  that, each of its names counts.
- A name that is not valid UTF-8 can't be shown: a file so named counts among `other_files`,
  and a folder so named counts, with all it holds, into its parent's `other_files`.
- It never crosses a filesystem boundary (a nested mount is skipped, like `du -x`) and never
  follows a symlink; symlinks and other special files count as their own small entry.
- `.explorer-uploads` is skipped, so staged data is not counted.
- A folder it cannot read is skipped and marks itself and every folder above it as
  `incomplete`: its total is a lower bound.
- A volume is walked by 4 threads. It keeps at most 200,000 folders as nodes; beyond that
  the deeper sub-folders are counted into their last listed ancestor and `collapsed` is set.
- It keeps at most 200,000 file names in all (the biggest files of the folders read first,
  usually the shallow ones); the folders read after that show their files as `other_files`
  only. Together these bound a tree to roughly 50 MB of memory.
- Folders are opened without following symlinks and checked against the inode seen when they
  were listed, so a folder swapped for a symlink meanwhile is not entered.
- Folders waiting to be read more than 64 levels down don't keep their parent open; they are
  reopened one name at a time from the scanned folder, so open descriptors don't grow with depth.

The tree is kept in memory with the task. A user keeps at most 4 finished scans (`MAX_USAGE_TREES`)
and one per folder: a new scan of a folder replaces the previous one, and the oldest beyond four
are dropped. A dismissed task, a task older than an hour and a restart lose their result, and
the usage route then answers 404 `task_not_found`, so the web app offers a new scan. Access is
checked again on every read of a result.

The scan is logged at `info` (`action` = `usage`, with `files`, `dirs`, `bytes`, `skipped` and
`elapsed_ms`); a failure at `warn`.

## Web app

Routes: `/` (opens the first volume, or says there is none), `/browse/<volume>/<path>`,
`/access-denied`, `/signin-error` and a not-found page. The folder lives in the URL, so
links, reload and the back button work. The sidebar lists the volumes with their free
space and a read-only marker. The folder is shown as a table sorted by name, size or date
(folders first), rendered through a virtualised list so a folder with tens of thousands of
entries keeps the DOM small. Sizes use SI units (1 kB = 1000 B). Each file row links to
`/files/download` and each folder row to `/files/zip`; these are plain links, so the
browser handles the download and shows any failure in its own download list.

Managing files: each row has a menu (Download, Rename, Move to, Copy to, Delete) and a checkbox;
checking rows, with shift-click for a range, shows a bar to download them as a zip (the names are stored
with `POST /files/zip/selections`, then the zip is fetched by its id, so thousands of entries
fit; a failure to store them shows a dismissable alert), move, copy or delete them, and renaming is offered for one entry at a time.
New folder is in the header. On a read-only volume only Download and Copy remain, and entries listed as `other` (symlinks the
server does not follow) can only be renamed or deleted. Names are checked
before they are sent (not empty, no slash, not `.` or `..`, at most 255 bytes, not the reserved
name at a volume root). Move and Copy open a folder picker that can cross volumes and cannot
choose a read-only one. A rename, move or copy that hits an existing name opens the same Replace, Keep
both or Cancel prompt as uploads, with "apply to all"; Replace is only offered for a file over
a file. Delete, Move and Copy become tasks that appear in the activity panel (with the uploads)
with their progress, a cancel or dismiss button and, for a task that skipped entries, how many.
One action shows at most 20 task rows there, then a count of the others (a delete of thousands
of entries is one task per entry).
The tab polls `GET /api/tasks` every second while one of its tasks is queued or running, and
refreshes the listing and the free space when one finishes. A tab shows the tasks it started;
on load it also lists the user's queued and running tasks once and adopts them, so a reload
keeps showing them until they finish (an adopted task never asks about a conflict, it is past
that step). A file task that drops out of the poll (another tab cleared it, it expired while
the tab slept, or the server restarted) is shown as no longer tracked, with an unknown outcome,
not as failed.

Polish: a refresh button next to the breadcrumb reloads the listing, the free space and the
favorites. The star next to the folder name adds or removes a favorite; favorites are listed
in the sidebar. The search field in the header (an icon that expands on phones) waits 300 ms
after the last keystroke, needs 2 characters and replaces the listing with the hits, each
linking to its folder; a "more results exist" note shows when the server truncated. A toggle
switches the folder between the list and a grid of tiles; images show a thumbnail and fall
back to the file icon when none is available. The choice is kept in `localStorage`
(`explorer.view`) and the grid is virtualised by rows. "Analyze space" on a folder queues a
usage task and opens `/usage/<task>/<volume>/<path>`, a bar list of sub-folders and big
files that opens a folder on click, with a link back to the listing; a folder row can also
show its size once a scan covers it. A scan that is closed in the activity panel keeps its
result for the tab until dismissed. Everything follows the system's light or dark setting
(`prefers-color-scheme`, `color-scheme: light dark`). On a read-only volume a notice says
that changes are disabled and the write controls are off or absent.

Uploads: the header has an Upload button (a floating "+" on narrow screens) that offers
files or a folder, and files or folders can be dropped on the folder listing. On a read-only volume
the controls are disabled. A panel lists the transfers with their progress, a cancel button
per item, and how many dropped or picked items could not be read and were skipped; retries
are automatic. Files dropped anywhere else in the page are ignored rather than opened by the
browser. The engine lives in `crates/web/src/uploads`:

- A file is cut into 16 MiB chunks, each sent as one `PATCH`; at most 3 files upload at
  once. Before creating an upload, the client resumes a pending one (from `GET /api/uploads`)
  whose volume, path, size and modification time all match, so reloading and picking the
  same file again continues from the server's offset. It resumes only an upload created with
  the `fail` policy, so that a new batch still asks before overwriting, and only when the
  fingerprints match: a 64-bit FNV-1a hash of the file's first and last 64 KiB
  (`shared::tus::fingerprint`), sent at creation. A different file with the same name, size
  and date is uploaded anew rather than appended to the first one's data.
- After a transient failure (network error, a 5xx answer that is not an API error, such as a
  proxy's 502, `internal`, `upload_busy`, `offset_mismatch`, `too_many_patches`) the item asks the server for its offset and retries after
  `min(1 s x 2^(attempt-1), 30 s)`, until the user cancels. A `PATCH` that reports no
  progress for 120 seconds is aborted and counts as a network error. An upload the server no
  longer knows (`upload_not_found`) is recreated from the start, once. When that offset is
  the full length, an empty `PATCH` confirms the commit before the item is marked done.
- Cancelling aborts the request in flight and deletes the upload. If the last chunk had
  already been committed (`upload_committed`), the item shows as done, not cancelled, and
  the listing refreshes.
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

One folder per domain under `crates/server/src`:

- `config` — loads and validates the environment at startup, reporting every problem.
- `db` — SQLite pool for `/config/explorer.db` (WAL), with a writability check.
- `telemetry` — `tracing` setup (pretty or JSON) and log value truncation.
- `healthcheck` — client used by `explorer healthcheck` for the Docker healthcheck.
- `http` — the plumbing every domain shares.
  - `mod` — shared state, middleware stack and the route tree composed from each domain's
    routes, `/healthz` and the JSON 404 fallback.
  - `error` — the JSON error type and the request id it carries.
  - `request_id` — the `X-Request-Id` header, its validation.
  - `static_files` — bundle check, SPA fallback and cache headers.
  - `security_headers` — CSP, `X-Content-Type-Options`, `Referrer-Policy`.
  - `access` — handler helpers: volume lookup, policy check, read-only rejection and path
    parsing in one call, and running a blocking storage call on a volume.
- `auth` — sign-in and access control.
  - `routes` — `/auth/*` handlers and `GET /api/me`.
  - `current_user` — the `CurrentUser` extractor.
  - `session` — cookie and SQLite-backed session layer.
  - `csrf` — `Origin`/`Referer` check on state-changing requests.
  - `policy` — authorization policy; handlers pass the user, the volume and the action, so
    finer rules can be added later.
  - `oidc` — provider discovery (retried at startup), authorization URL, code exchange, ID
    token verification, groups, end-session URL.
- `storage` — all filesystem access; handlers never touch the filesystem directly.
  - `path` — `RelPath`, the validated relative path type.
  - `volumes` — discovery under `/data` and read-only detection.
  - `read` — listing, `stat`, opening files, walking a folder, free space. Blocking calls
    on [cap-std](https://docs.rs/cap-std) directories, run on Tokio's blocking pool.
  - `staging` and `write` — the per-volume staging directory and its files, the upload
    target checks, folder creation, and `place`, the rename into place that the rename, move
    and upload commits share.
  - `rename` and `delete` — renaming an entry in place, and removing a file, symlink or
    folder tree.
  - `transfer` — copy and move: `check` (can it be done now, no folder into itself),
    `copy` (staged files, plan and progress), `move` (rename, or copy then delete the source).
  - `search` — breadth-first name search with a hit limit, a deadline and cancellation.
  - `usage` — the space scan: `scan` (parallel walk of a folder by file descriptor) and the
    tree it builds, with the per-folder views.
  - `error` — storage errors, mapped to API error codes in `http::error`.
- `browse` — `GET /api/volumes`, `GET /api/volumes/{volume}/entries`,
  `POST /api/volumes/{volume}/folders` and `POST /api/volumes/{volume}/rename`.
- `downloads` — `/files/download` (with `Range`) and `/files/zip` (a folder, or named entries
  of one).
- `tasks` — background delete, copy and move.
  - `routes` — `/api/tasks`: checks a task, queues it, lists, cancels or dismisses.
  - `registry` — the in-memory tasks per user, their `Progress` and the expiry of finished
    ones.
  - `run` — the queue: a semaphore of 2 permits and a blocking thread per running task.
  - `transfer` — the copy and move tasks, built on `storage`.
  - `usage` — the usage task (access check, scan, log) and `GET /api/tasks/{id}/usage`.
- `favorites` — `routes` (`/api/favorites`) and `store` (the `favorites` table, per user, capped at 100).
- `search` — `GET /api/volumes/{volume}/search`: validation, cancellation when the request is
  dropped, DTO mapping; the walk is `storage::search`.
- `thumbnails` — `GET /files/thumbnail`.
  - `routes` — access, `ETag`, headers, the render permit.
  - `render` — decoding within limits, orientation, shrinking and re-encoding.
  - `cache` — the files under `/config/cache/thumbnails`, their key, the touch and the sweep.
- `uploads` — resumable uploads.
  - `routes` — `GET /api/uploads`, the user's pending uploads.
  - `tus` — the `/files/uploads` tus endpoints: `protocol` (headers, version check,
    creation metadata), `handlers`, `staged` (the staged file) and `commit` (writing the
    body and moving the file into place).
  - `store` — the `uploads` table and in-memory locks.
  - `cleanup` — the abandoned-upload sweep.

## Web modules

Under `crates/web/src`:

- `app`, `routes`, `i18n`, `format`, `listing` — the root component, the client routes,
  translations, size and date formatting, sorting, file types and the visible row range.
- `api` — fetch helpers and API failures; `urls` builds the API and file URLs.
- `tasks` — the background task list of the tab: `store` (state machine: tasks, events,
  effects, polling), `conflict` (the existing entry a copy or move would land on),
  `usage` (the scans started from this tab and their results), `transport` (the requests).
- `favorites` (the user's favorites, reloaded with the revision), `search` (the query shared by
  the header field and the browse page), `view_mode` (list or grid, remembered).
- `selection` (checked rows), `actions` (the row and selection actions by entry kind and
  volume), `names` (name validation), `conflict` (the Replace, Keep both or Cancel decision
  shared with uploads), `revision` (a counter bumped when something changed on disk, so the
  listing and sidebar reload).
- `uploads` — the upload engine: `plan` (what a pick or drop sends), `queue` (the state
  machine: items, effects and events, the reducer), `controller`, `transport`, `drop`,
  `speed`.
- `components` — grouped by screen: `layout` (shell, sidebar, breadcrumb, user menu,
  sign-out, activity panel), `browse` (file table, grid and tiles, rows, row menu, selection bar,
  refresh and view toggle, read-only notice, virtual list, drop zone), `manage` (new folder, rename, delete and move or copy dialogs, folder
  picker), `favorites` (sidebar list, star), `search` (header field, results), `usage` (analyze
  button, folder size, header and bar list), `tasks` (task rows, conflict dialog), `uploads` (upload button, upload rows,
  conflict dialog), `common` (icons, menu, modal, conflict prompt, notice, error panel).
- `pages` — one component per route (`browse`, `usage`, ...).
