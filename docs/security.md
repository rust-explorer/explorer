# Security

This page describes what Explorer protects against as of the browsing, download,
upload and file management milestone. To report a vulnerability, follow [SECURITY.md](../SECURITY.md); do not open a
public issue.

## Scope

Explorer sits behind a reverse proxy that terminates TLS and delegates identity to an OIDC
provider. It defends against: a malicious website acting on behalf of a signed-in user,
stolen or forged sign-in responses, token leakage to the browser, and users outside the
allowed group. It also defends the volumes against path tricks (traversal, symlinks that
point outside a volume) and keeps the files it serves from running in the app's origin.
It does not defend against a compromised provider, reverse proxy or host, nor against
someone who can already write inside a volume on the host. Every signed-in user in the
allowed group can read every volume and write to every volume that is not read-only;
per-volume or per-folder permissions do not exist yet.

## Tokens stay on the server

Explorer is a backend-for-frontend. The server performs the OIDC authorization code flow
with PKCE (S256), `state` and `nonce`, and verifies the ID token (signature, issuer,
audience, expiry, nonce, access token hash). Access, ID and refresh tokens are never sent
to the browser and never logged. The browser only holds a session cookie.

## Session cookie

- Name `explorer_session`; `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
- Session data lives server-side in SQLite (`/config/explorer.db`); the cookie is an
  opaque identifier.
- Expiry is sliding: `EXPLORER_SESSION_TTL` of inactivity (default 7 days, capped at 400).
- The session id is rotated on successful login (against session fixation).
- Logout and access denial flush the session. After a denial only the ID token is kept,
  solely so that signing out can end the provider session.

Because the cookie is `Secure`, Explorer must be served over HTTPS.

## CSRF

`SameSite=Lax` is the first layer. In addition, every request other than `GET`, `HEAD` and
`OPTIONS` must carry an `Origin` (or, failing that, `Referer`) header whose origin equals
the origin of `EXPLORER_PUBLIC_URL`; otherwise the server answers 403 `csrf_rejected`.
Login is a harmless `GET` that only starts a sign-in; the callback is protected by the
`state` parameter, which is bound to the session and usable once.

This covers every write: creating, appending to and cancelling an upload (`POST`, `PATCH`,
`DELETE` under `/files/uploads`), creating a folder, renaming, and creating or cancelling a
task (`POST /api/tasks`, `DELETE /api/tasks/{id}`). The zip of several entries is a `GET` and
changes nothing. Uploads use same-origin `XHR` and
`fetch`, so the browser sends an `Origin` the check accepts.

## Open redirects

`/auth/login?return_to=` must be a path (leading `/`, no control characters, at most 2048
bytes). It is resolved against `EXPLORER_PUBLIC_URL` with a WHATWG URL parser, the same rules
browsers apply, and kept only if the result has the same origin and its path does not start
with `//`. So `//host`, `/\host` or `/..//host` cannot send the user to another site; they
become `/`. The post-login redirect target is stored in the session, never taken from the
callback.

## Groups gate

A user can sign in only if `EXPLORER_ALLOWED_GROUP` is in the groups claim
(`EXPLORER_OIDC_GROUPS_CLAIM`, ID token first, userinfo as fallback). A missing or
malformed claim fails the sign-in; it never grants access. Groups are read at sign-in
only; removing someone from the group takes effect when their session ends (log out, or
expiry), not immediately.

## Path confinement

Clients name a file by `(volume, path)`. The path is parsed into a `RelPath` before any
filesystem access, and the server rejects with `invalid_path` (400) a path that:

- starts with `/`;
- is longer than 4096 bytes, or has a component longer than 255 bytes;
- contains a NUL or any other control character;
- has a `.` or `..` component.

Empty components (`a//b`, a trailing `/`) are dropped. Backslashes are ordinary
characters. A volume name is only looked up in the list discovered at startup, so it can
never be a path.

Parsing is not the security boundary; confinement is. Each volume is opened once as a
[cap-std](https://docs.rs/cap-std) `Dir`, and every access resolves from that directory.
On Linux this uses `openat2` with `RESOLVE_BENEATH`, so the kernel refuses any resolution
that leaves the volume, including through a symlink or a concurrent rename. Such an
attempt is answered with 403 `outside_volume` and logged at `warn`. All filesystem access
on user data goes through the `storage` module.

A volume is a real directory directly under `/data`. A symlink there is not a volume.
Names that are not valid UTF-8 are skipped with a warning.

## Symlinks

- A relative symlink that stays inside its volume is followed, for listings and downloads.
- An absolute symlink, a broken one, and one that resolves outside the volume are listed
  as `other` (no size, no date), with a disabled row in the UI, and are never read. This
  holds even when the absolute target would happen to lie inside the volume.
- Opening one by path gives `outside_volume` or `path_not_found`, never its content.
- Folder downloads never descend into a symlink to a directory, which also rules out
  cycles, and skip anything that is neither a regular file nor a directory (sockets,
  FIFOs, devices). Symlinks to files inside the volume are included as files.
- Files are opened non-blocking, so a FIFO swapped in after the check cannot hang a
  request, and the type is checked again on the open file.

## Uploads

- **Confinement.** A write goes through the same `(volume, path)` parsing and cap-std
  confinement as a read, so a symlink or `..` cannot make an upload land outside its
  volume. A write to a read-only volume is refused with 403 `read_only_volume` before
  anything is touched.
- **Staying in the volume.** Incoming data is staged in `.explorer-uploads` at the root of
  the volume it is destined for and moved into place with a rename relative to open
  directory handles, so no file content is copied across volumes or through a temporary
  directory elsewhere. The staging directory is opened without following symlinks and must
  be a real directory; the staged file is created with `O_NOFOLLOW` in a `0700` directory,
  so it stays private while it is incomplete. It is created with mode `0666` less the
  container's umask and the rename keeps that mode, so an uploaded file, like the folders an
  upload creates, gets the same permissions as any file the container creates (`0644` with
  the usual `022` umask: readable by other host users who can reach the folder). To keep
  uploads private, restrict access to the volume's folder on the host.
- **Reserved name.** Any client path whose first component is `.explorer-uploads` is
  rejected with `invalid_path` (reason `reserved name`). The volume root and the staging
  directory are also recognised by device and inode, so a symlink inside the volume that
  points to the root or to the staging directory does not reach it either: it is never
  listed or zipped, and reading or writing through such a path is `path_not_found`.
- **Ownership.** An upload belongs to the OIDC `sub` that created it. Looking it up, resuming
  it, appending to it or cancelling it as another user is a 404 `upload_not_found`; the
  upload id is 128 random bits and is not a capability on its own. `GET /api/uploads` lists
  only the caller's uploads. Write access to the volume is checked again on every append.
- **No silent overwrite.** The default conflict policy is `fail`. The final rename uses
  no-replace semantics for `fail` and `keep_both`, so a file created while the upload was
  running is never replaced; only an explicit `replace` overwrites, and never a folder.
  The client's `modified` time is applied to the final file; nothing else from the client's
  metadata is trusted.
- **Limits.** The declared `Upload-Length` is checked against the volume's free space at
  creation and a body longer than the declared length is cut off (413 `upload_too_large`).
  Disk-full during a write is 507 `insufficient_storage`. A body that stalls for 60 seconds
  is dropped. Each upload is written by one request at a time (423 `upload_busy`).
- **Cleanup.** An upload untouched for 24 hours is removed with its staged file, as is a
  stale staged file with no record. The sweep runs at startup and every hour.
- **Logs.** Uploads are logged at `info` with the user's `sub`, the volume and the path (and
  `bytes` and `final_path` on completion); folder creation and sweeps likewise; failures at
  `warn`. File contents are never logged.

## Managing files

New folder, rename, delete, copy and move reuse the confinement of the sections above: every
path goes through `(volume, path)` parsing and cap-std, and a symlink or `..` cannot make an
action reach outside its volume.

- **Write access.** Creating a folder, renaming, deleting and moving need write access to
  the volume, checked before anything is touched (403 `read_only_volume`); a move checks it on
  the source volume too, since it removes from it. Copying needs only to read the source, and
  write access to the destination volume. Creating a task checks all of this up front.
- **Reserved paths.** `.explorer-uploads` is refused as a path, as a new name and as a
  destination, wherever it is reached from (including through a symlink to the volume root).
  It cannot be renamed, deleted, copied, moved or zipped. The volume root itself cannot be
  renamed, deleted, copied or moved.
- **Symlinks.** Delete and rename act on the symlink itself and never follow it: deleting a
  symlink removes the link, not its target. A move within a volume renames the link, and a
  copy, like a move across volumes, reads through a relative symlink that stays
  inside the volume, as a download does, so the destination gets a regular file or folder
  (a symlink to a folder inside a copied folder is skipped and counted as `skipped`, which
  also rules out cycles, as do sockets, FIFOs and devices); a move across volumes then removes
  only the link from the source. An absolute, broken or escaping symlink (listed as `other`)
  cannot be the source of a copy or move: `outside_volume` for an absolute or escaping one,
  `path_not_found` for a broken one. Deleting or renaming one is
  allowed, since that touches only the link.
- **No silent overwrite.** The conflict policy defaults to `fail`. Replace is only for a file
  over a file: it is refused (`is_a_directory`) when the source or the existing target is a
  folder, and the web app does not offer it then. A copied folder is created under a free
  name and is never merged into an existing folder, even one that appeared after the check.
  A rename never replaces by accident: with `fail` and `keep_both` it uses a no-replace
  rename, so a name taken meanwhile is not overwritten (with `keep_both`, or with `fail` when
  the name appeared after the check, the entry gets a "keep both" name: `Film (1).mkv`).
  A move within a volume is one no-replace rename under the same rules.
- **Into itself.** Copying or moving a folder into itself or one of its descendants is
  refused (400 `invalid_destination`). The destination is walked up by device and inode, so a
  symlink leading into the folder is caught as well; a symlink to the volume root as source
  is treated as the root. Moving an entry into the folder it already is in is refused too.
  The error's `details` name the entry being moved or copied, on its source volume.
- **Cross-volume moves.** Volumes may be different filesystems, so a move to another volume
  copies first, then deletes. The source is touched only after the copy is complete: a
  failed or cancelled copy leaves it whole and removes the partial destination. After that,
  each copied file is deleted from the source, then the folders left empty. What the copy
  skipped (see Symlinks) stays in the source, with its folders, and is reported in `skipped`.
  If a delete fails midway, the destination is complete and the source may be partly
  removed; nothing is lost, but the move is reported as failed. A move within one volume that hits a nested mount falls back to the same copy and delete.
  Copies use the staging directory of the destination volume, so their data never leaves it
  until it is renamed into place.
- **Tasks.** A task belongs to the OIDC `sub` that created it; listing, cancelling or
  dismissing another user's task is a 404 `task_not_found`, and the ids are 128 random bits.
  Tasks are kept in memory only: a restart loses them, running ones included, and a
  delete or move interrupted that way may be half done, and a copy may leave a partial
  folder or a staged file in `.explorer-uploads` (the hourly sweep removes the staged file
  after 24 hours). At most 2 tasks run at
  once for the whole server, so a large copy delays others. A finished task is forgotten after
  an hour. Failures keep the request id of the request that created the task.
- **Zip of several entries.** Each `name=` must be a single valid component: empty, `.`,
  `..`, containing `/`, a control character, or the reserved name at the volume root is
  `invalid_path`, and a
  name that does not exist is `path_not_found`. Entries are resolved and walked under the
  same rules as a folder zip, and need read access only.
- **Logs.** A new folder, rename, delete and finished copy or move are logged at `info`
  with the user's `sub`, `action`, `volume`, `path` (and `dest_volume` and `final_path`);
  failures, refused ones included, once at `warn` with the same fields and the error. A
  cancelled or dismissed task is logged at `info` with the user's `sub`, `action` (`cancel`
  or `dismiss`) and the task id. File contents are never logged.

## Downloads

User files are never rendered by the browser in the app's origin. Every download (file and
zip) carries:

- `Content-Type: application/octet-stream`, whatever the extension;
- `Content-Disposition: attachment`, with an ASCII fallback `filename` and an RFC 8187
  `filename*` for the real name;
- `Content-Security-Policy: sandbox`;
- `X-Content-Type-Options: nosniff`.

The file endpoint answers a single byte range (`206`), advertises `Accept-Ranges: bytes`
and sends a strong `ETag` and a `Last-Modified`. `If-Range` is compared strongly: a weak
tag never matches, and a date matches only if it equals the file's modification time
and that time is at least one second old. When it does not match, the full file is sent.
Malformed ranges and multi-range requests also get the full file; a range that starts past
the end is `416 range_not_satisfiable`.

Downloads and zips are logged at `info` with the user's OIDC `sub`, the volume, the path
and (for zips) the number of files and bytes. Listings are logged at `debug`, storage
failures at `warn` with the code, the volume and the path. File contents are never logged.
Only an unexpected I/O failure (`internal`) also logs the OS error, which is not sent to
the client.

## Content Security Policy and headers

Every app response carries a CSP of `default-src 'self'; script-src 'self' 'wasm-unsafe-eval';
style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none';
base-uri 'self'; form-action 'self'`, plus `X-Content-Type-Options` and `Referrer-Policy`.
Download and zip responses replace that CSP with `Content-Security-Policy: sandbox` (see
[Downloads](#downloads)).
Sign-in did not loosen it. The provider is reached by top-level redirects only.

## Secrets and logs

The client secret can be supplied as a file (`EXPLORER_OIDC_CLIENT_SECRET_FILE`) and is
never logged. Sign-in failures log a reason and the provider's error text (truncated);
rejected cross-origin requests log the offending origin (truncated). Tokens, codes,
session ids and the client secret are not logged. Provider requests do not follow
redirects, and discovery failures are retried. A discovery document that fails
validation (for example an issuer mismatch) stops startup; it is never used.
