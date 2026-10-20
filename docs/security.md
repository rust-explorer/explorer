# Security

This page describes what Explorer protects against as of the browsing, download and
upload milestone. To report a vulnerability, follow [SECURITY.md](../SECURITY.md); do not open a
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
`DELETE` under `/files/uploads`) and creating a folder. Uploads use same-origin `XHR` and
`fetch`, so the browser sends an `Origin` the check accepts.

## Open redirects

`/auth/login?return_to=` accepts only same-site absolute paths (a single leading `/`, no
`//` or `/\`, no control characters, at most 2048 bytes). Anything else becomes `/`. The
post-login redirect target is stored in the session, never taken from the callback.

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
  be a real directory; the staged file is created with `O_NOFOLLOW`, mode `0600`, in a
  `0700` directory. The rename keeps the file's mode, so an uploaded file ends up `0600`
  and owned by the container user (not readable by other host users or groups); folders
  created by an upload get the container's umask.
- **Reserved name.** `.explorer-uploads` is never listed or zipped, and any client path
  whose first component is that name is rejected with `invalid_path` (reason `reserved
  name`), so it can be neither read, written nor created through the API.
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
