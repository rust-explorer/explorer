# Security

This page describes what Explorer protects against. To report a vulnerability, follow
[SECURITY.md](../SECURITY.md); do not open a public issue.

## Scope

Explorer sits behind a reverse proxy that terminates TLS and delegates identity to an OIDC
provider. It defends against: a malicious website acting on behalf of a signed-in user,
stolen or forged sign-in responses, token leakage to the browser, and users outside the
allowed group. It also defends the volumes against path tricks (traversal, symlinks that
point outside a volume) and keeps the files it serves from running in the app's origin.
It does not defend against a compromised provider, reverse proxy or host, nor against
someone who can already write inside a volume on the host. Every signed-in user in the
allowed group can read every volume and write to every volume that is not read-only;
there are no per-volume or per-folder permissions.

## Tokens stay on the server

Explorer is a backend-for-frontend. The server performs the OIDC authorization code flow
with PKCE (S256), `state` and `nonce`, and verifies the ID token (signature, issuer,
audience, expiry, nonce, access token hash). Access and refresh tokens are never sent to
the browser, and no token is logged. The browser only holds a session cookie.

The one exception is signing out: the ID token is kept in the session and sent once, as
`id_token_hint` in the provider's end-session URL, so that the provider can end its own
session and redirect back without asking. That URL passes through the browser, so the ID
token (subject, name, email, groups; already expired or about to) can appear in the
browser history and in the access logs of the reverse proxy and the provider.

The provider's signing keys (JWKS) are fetched at startup. When an ID token names a key
Explorer does not know, as after a key rotation at the provider, Explorer fetches the keys
again, at most once a minute, and verifies the token once more; no restart is needed.

## Session cookie

- Name `explorer_session`; `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
- Session data lives server-side in SQLite (`/config/explorer.db`); the cookie is an
  opaque identifier. The database stores session ids in clear, so it is created readable
  by Explorer's user only (mode `600`, `-wal` and `-shm` included); keep `/config` at `700`.
- Expiry is sliding: a session ends after `EXPLORER_SESSION_TTL` of inactivity (default
  7 days, capped at 400). Activity pushes the expiry back at most once an hour (or every half
  TTL, if shorter), so an idle session may end up to that much earlier. Only requests under
  `/api`, `/files` and `/auth` read the session; static files and `/healthz` never touch the
  database. A request that pushed the expiry back does not save it if the session was signed
  out while it ran, so it cannot bring that session back.
- However active, a session ends 30 days after sign-in, or after `EXPLORER_SESSION_TTL` if
  that is longer. The user then signs in again, and their groups are read again.
- A sign-in in progress (its `state`, `nonce`, PKCE verifier and return path) is kept in a
  session that lasts 10 minutes, since anyone can start one; expired sessions are deleted
  every 10 minutes. Up to 5 sign-ins can be in progress at once in one browser, so tabs that
  all found their session ended can sign in together; a sixth drops the oldest. A callback
  whose `state` matches none of them cancels them all.
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
task (`POST /api/tasks`, `DELETE /api/tasks/{id}`), and adding or removing a favorite (`PUT`, `DELETE /api/favorites`). The zip of several entries is a `GET` and
changes nothing; storing a large selection for it (`POST /files/zip/selections`) is a
`POST` and goes through this check. Uploads use same-origin `XHR` and
`fetch`, so the browser sends an `Origin` the check accepts.

Everything under `/files` (downloads, zips, thumbnails, uploads) also refuses, with 403
`csrf_rejected`, a request the browser marks `Sec-Fetch-Site: cross-site` or `same-site`, so that
an image or a script on another site, sibling subdomains included (other apps on the same NAS
domain), cannot start a download or a zip with the user's cookie. A `GET` the user started by
clicking a link (`Sec-Fetch-Mode: navigate` with `Sec-Fetch-User: ?1`), such as a download link
in a webmail, still passes. A request without these headers passes too.

The JSON API under `/api` applies the same rule, reads included: the web app calls it from its
own origin only, and a page on a sibling subdomain must not be able to keep searches or large
listings running with the user's cookie, even though it cannot read the answers.

## Open redirects

`/auth/login?return_to=` must be a path (leading `/`, no control characters, at most 512
bytes; a longer one becomes `/`). It is resolved against `EXPLORER_PUBLIC_URL` with a WHATWG URL parser, the same rules
browsers apply, and kept only if the result has the same origin and its path does not start
with `//`. So `//host`, `/\host` or `/..//host` cannot send the user to another site; they
become `/`. The post-login redirect target is stored in the session, never taken from the
callback.

## Sign-in rate limit

`/auth/login` and `/auth/callback` answer anyone, and each call costs a session write and,
for the callback, requests to the provider. They share one limit per client address: a
burst of 10 requests, then 20 a minute. Beyond it they answer 429 `too_many_requests` with
`Retry-After` (a browser is sent to `/signin-error?reason=too_many_requests`), and the
server logs `too many sign-in requests` at warn with the request id and the address. The
rest of the API requires a session and is not limited this way.

The client address is the TCP peer, unless the peer is in `EXPLORER_TRUSTED_PROXIES`: then
it is the rightmost `X-Forwarded-For` entry that is not a trusted proxy, so a client cannot
pick its own address by sending the header through the proxy. By default the trusted peers
are the loopback and private networks, where the reverse proxy runs; a host on those
networks talking to Explorer directly can therefore choose the address it is counted
under, which only gives it a bucket of its own. On the Internet side, a request with an
unreadable or forged header part is still counted under the address the proxy appended.
When a peer outside the list sends `X-Forwarded-For`, the server logs it once at warn.

IPv6 clients are counted per /64, the block one subscriber usually gets. The limiter tracks at most 10 000 addresses in memory; an address whose allowance has fully
refilled is forgotten, and when all 10 000 are active the one closest to refilled is
dropped.

## Groups gate

A user can sign in only if `EXPLORER_ALLOWED_GROUP` is in the groups claim
(`EXPLORER_OIDC_GROUPS_CLAIM`, ID token first, userinfo as fallback). A missing or
malformed claim fails the sign-in; it never grants access. Groups are read at sign-in
only; removing someone from the group takes effect when their session ends (log out,
inactivity, or at most 30 days after sign-in, or `EXPLORER_SESSION_TTL` if longer), not
immediately.

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
- Folder downloads never descend into a symlink to a directory found inside the zipped
  folder, which also rules out cycles: it is left out and listed in `explorer-skipped.txt`,
  as is anything that is neither a regular file nor a directory (sockets, FIFOs, devices).
  A symlink to a folder inside the volume that the user selected itself in a multi-entry
  download is followed, that one level only. Symlinks to files inside the volume are
  included as files.
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
- **Reserved name.** Any client path whose first component is `.explorer-uploads`, in any
  letter case (a volume may sit on a filesystem that ignores case), is rejected with
  `invalid_path` (reason `reserved name`), and no such entry is listed, zipped or counted
  in space usage at a volume root. A folder of that name made outside Explorer at a volume
  root (say `.Explorer-Uploads` on a case-sensitive filesystem) is therefore out of reach
  from Explorer; rename it on the host to reach it. The volume root and the staging
  directory are also recognised by device and inode, so a symlink inside the volume that
  points to the root or to the staging directory does not reach it either: it is never
  listed or zipped, and reading or writing through such a path is `path_not_found`. The
  check is made again on the folder handle the operation then acts through, so a symlink
  renamed in and out of place while a request runs cannot slip past it.
- **Ownership.** An upload belongs to the OIDC `sub` that created it. Looking it up, resuming
  it, appending to it or cancelling it as another user is a 404 `upload_not_found`; the
  upload id is 128 random bits and is not a capability on its own. `GET /api/uploads` lists
  only the caller's uploads. Write access to the volume is checked again on every append.
- **No silent overwrite.** The default conflict policy is `fail`. The final rename uses
  no-replace semantics for `fail` and `keep_both`, so a file created while the upload was
  running is never replaced; only an explicit `replace` overwrites, and never a folder.
  The client's `modified` time is applied to the final file; nothing else from the client's
  metadata is trusted. The `fingerprint` is only stored and returned to its owner, who uses it
  to avoid resuming an upload with another file's data.
- **Limits.** A user has at most 1000 unfinished uploads; more are refused with 429
  `too_many_uploads` until some finish, are cancelled or are swept. The declared
  `Upload-Length` is checked at creation against the volume's free space less the bytes
  that the unfinished uploads (all users) to every volume on the same filesystem (same
  device at startup, e.g. two folders of one disk) still have to receive, so many large
  uploads cannot together promise more than the disk holds. A body longer than the declared
  length is cut off (413 `upload_too_large`).
  Disk-full during a write is 507 `insufficient_storage`. A body that stalls for 60 seconds
  is dropped. Each upload is written by one request at a time (423 `upload_busy`), and a
  user has at most 8 `PATCH` requests in flight (429 `too_many_patches`, which the web app
  retries), so a client that trickles bytes ties up a bounded number of open files.
- **Cleanup.** An upload untouched for 24 hours is removed with its staged file, as is a
  stale staged file with no record. The sweep runs at startup and every hour. A new upload
  replaces the same user's unfinished uploads to the same volume and path that had no
  `PATCH` for 2 minutes: the client starts a new upload only when it will not resume the
  old one (after a crash, with another conflict choice, or for another file), so the old
  one's staged file and record are removed and its space is freed at once. One written to
  more recently may still be running in another tab and is kept.
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
  failed or cancelled copy leaves it whole and removes the partial destination: only
  the files and folders the copy made, each checked by device and inode (and size and
  modification time for files), so a folder renamed into its place, or files someone added
  to it meanwhile, stay. After that,
  each copied file is deleted from the source, then the folders left empty. What the copy
  skipped (see Symlinks) stays in the source, with its folders, and is reported in `skipped`.
  A source file is deleted only if it is still the file that was read: same device and
  inode, size, modification and change time as when the copy opened it. One that was
  written to, replaced or renamed over since stays in the source next to its copy, and the
  move fails with `changed_during_move` (409). A symlink to a file, copied as the file it
  leads to, is checked and deleted as the link itself (same device, inode and modification
  time), so it goes even once its target was moved, while its target stays unless moved
  too. For a file with several hard links in the moved folder, deleting one name moves the
  change time of the others; that change, made by the move itself, is not counted. Only a change in the instant between that
  check and the delete can slip through.
  If a delete fails midway, the destination is complete and the source may be partly
  removed; nothing is lost, but the move is reported as failed. A move within one volume that hits a nested mount falls back to the same copy and delete.
  Copies use the staging directory of the destination volume, so their data never leaves it
  until it is renamed into place.
- **Size of a copy.** A copy, or a move to another volume, first lists the whole folder,
  which it keeps in memory while it runs: past 1,000,000 files and folders it stops and fails
  with `too_many_entries` before copying anything. Cancelling the task also stops the listing.
- **Permissions.** A copy, and a move to another volume, give each file and folder the
  permission bits of its source (read, write and execute for owner, group and others), so a
  private `0600` file stays private; set-user-ID, set-group-ID and sticky bits are dropped.
  The owner and group are the container user's, as for any file it creates, and the owner
  always keeps read and write on files and read, write and search on folders (a `0555`
  folder is copied as `0755`, a `0444` file as `0644`), so Explorer can still read, change
  and delete its copy. A move within a
  volume is a rename and keeps everything. On a filesystem without Unix permissions (exFAT,
  some SMB mounts) the copy gets whatever that filesystem gives.
- **Tasks.** A task belongs to the OIDC `sub` that created it; listing, cancelling or
  dismissing another user's task is a 404 `task_not_found`, and the ids are 128 random bits.
  A user has at most 1000 tasks queued or running; more answer 429 `too_many_tasks`. The web
  app keeps such tasks waiting on its side and posts them as earlier ones finish, so a
  larger selection is moved, copied or deleted in full while the tab stays open.
  Tasks are kept in memory only: a restart loses them, running ones included, and a
  delete or move interrupted that way may be half done, and a copy may leave a partial
  folder or a staged file in `.explorer-uploads` (the hourly sweep removes the staged file
  after 24 hours). At most 2 tasks run at
  once for the whole server, so a large copy delays others. A finished task is forgotten after
  an hour, or sooner once its user has 1000 newer finished tasks, so a script posting tasks
  in a loop cannot grow the server's memory without bound. Failures keep the request id of the request that created the task.
- **Zip of several entries.** Each `name=` must be a single valid component: empty, `.`,
  `..`, containing `/`, a control character, or the reserved name at the volume root is
  `invalid_path`, and a
  name that does not exist is `path_not_found`. Entries are resolved and walked under the
  same rules as a folder zip, and need read access only. A selection stored with
  `POST /files/zip/selections` is checked the same way when stored and again when zipped,
  belongs to the user who stored it (another user gets `selection_expired`), expires after
  10 minutes, and a user keeps at most 16.
- **Zip walk.** Below the zipped folder, every folder and file is opened by its name from
  its already opened parent, without following symlinks, and a folder is only entered if it
  is still the one that was listed (device and inode). Symlinks to files are resolved from
  the volume root and confined like any path; symlinks to folders are not followed, unless
  the user selected the link itself in a multi-entry download (then only that link is
  followed; links to folders below it are listed in `explorer-skipped.txt`). Folders
  more than 64 levels down don't keep a descriptor open while they are walked. At most 4
  zips stream at once, 2 per user, so one signed-in user cannot hold every slot; each stops
  when its client goes away or stops reading for 60 seconds.
- **Logs.** A new folder, rename, delete and finished copy or move are logged at `info`
  with the user's `sub`, `action`, `volume`, `path` (and `dest_volume` and `final_path`);
  failures, refused ones included, once at `warn` with the same fields and the error. A
  cancelled or dismissed task is logged at `info` with the user's `sub`, `action` (`cancel`
  or `dismiss`) and the task id. File contents are never logged.

## Favorites, search, thumbnails and space usage

These features only read. They go through the same `(volume, path)` parsing and cap-std
confinement as everything else, work on read-only volumes, and check read access to the
volume on every request.

- **Favorites** belong to the OIDC `sub` that created them: the list, add and remove routes only
  see the caller's rows, and the limit of 100 is per user. A favorite is a bookmark, not a
  grant: opening it is an ordinary listing, so it needs read access then, and a path that
  has become a symlink escape is still refused. The path is validated and must be an existing
  folder when it is added.
- **Search** is confined to the volume it is called on. It lists the entries of each folder
  without opening anything, never descends into a symlinked folder (so no cycle and no
  escape), never matches an entry listed as `other` (absolute, broken or escaping symlinks,
  sockets, FIFOs, devices) and skips `.explorer-uploads`. A query needs 2 characters, the
  walk is capped at 200 hits and 20 seconds and stops when the client goes away, so one
  request cannot keep a thread for long. At most 4 searches walk at once across all users;
  more wait for one to end. The query is only logged at `debug`.
- **Space usage** walks a folder with directory file descriptors opened with `O_NOFOLLOW`
  and checks each folder's device and inode again when it opens it, so a folder swapped for a
  symlink during the scan is not entered. It never follows a symlink, never crosses to another
  filesystem, and skips `.explorer-uploads`. It reads metadata only, never file contents. A
  folder it cannot read marks the result `incomplete` instead of failing. A scan is a task, so it
  belongs to the `sub` that started it, shares the limit of 2 running tasks, and its result
  (at most 4 per user, one per folder) lives in memory only; reading it checks access to the
  scanned folder again. The totals show how much space a folder takes, which every signed-in
  user could already compute by listing it.

### Thumbnails

An image is user-controlled input fed to a decoder, so thumbnails are made defensively and
never serve the original:

- **Re-encoded.** The response is a fresh JPEG or PNG made from the decoded pixels, at most
  320 px on a side; metadata (EXIF, ICC, comments) is not copied. It is served inline, as an
  `<img>` source, which is why it is not a download.
- **Limits.** Only `jpg`, `jpeg`, `png`, `gif` and `webp` names are tried, and the content must
  actually be one of those formats (a renamed file is refused). A source over 64 MiB is
  refused, and at most 64 MiB are read even if the file grows meanwhile. The decoder is given
  limits of 16,384 px per side and 512 MiB of allocation. Before decoding, the memory the
  decode will take is estimated and an image over 512 MiB is refused: the source (read whole)
  plus the decoded size, counted three times for a progressive JPEG and for WebP and twice
  for GIF, whose decoders keep coefficients, planes or a frame besides the output. Measured
  with images just under the limit (baseline and progressive JPEG, WebP lossy and lossless,
  GIF), one decode peaks at 525 MiB of resident memory at most, so the 2 concurrent decodes
  stay near 1 GiB; a 16,384 x 10,922 progressive JPEG, which used to take 1.6 GB, is refused. Anything refused or undecodable is 404 `thumbnail_unavailable`
  and logged at `debug`; a source the server cannot open is refused like a download
  (403 `permission_denied`), also logged at `debug` only, since a folder of such images
  would log one line per tile.
- **Concurrency.** At most 2 decodes run at once across all users, each on the blocking pool.
  The permit stays with the work when the request is cancelled, so scrolling through a large
  grid cannot start more decodes than that. A request waiting for a permit has only looked the
  source up: the file is opened once the permit is granted (and refused if its size or
  modification time changed meanwhile), so a flood of queued requests holds no descriptors.
- **Headers.** `Content-Type` is the format that was written (`image/jpeg` or `image/png`),
  with `X-Content-Type-Options: nosniff` and `Content-Security-Policy: sandbox`, like downloads.
  `Cache-Control: private, max-age=604800` keeps it out of shared caches; the strong `ETag`
  is derived from the volume, path, size and modification time.
- **Cache.** Thumbnails are files under `/config/cache/thumbnails`, named by a hash that
  includes the volume, path, size, modification time and a rendering version; they hold no
  user name or path. They are not access-controlled by themselves: every request checks read
  access to the source first, then looks the thumbnail up. Entries untouched for 30 days
  are swept.

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
[Downloads](#downloads)); thumbnails send the sandbox policy too (see [Thumbnails](#thumbnails)). `img-src` allows `'self'`, so grid tiles load.
Sign-in needs no exception: the provider is reached by top-level redirects only.

## Secrets and logs

The client secret can be supplied as a file (`EXPLORER_OIDC_CLIENT_SECRET_FILE`) and is
never logged. Sign-in failures log a reason and the provider's error text (truncated);
rejected cross-origin requests log the offending origin (truncated). Tokens, codes,
session ids and the client secret are not logged. Provider requests do not follow
redirects, and discovery failures are retried. A discovery document that fails
validation (for example an issuer mismatch) stops startup; it is never used.
