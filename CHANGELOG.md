# Changelog

## 1.0.0

First public release. Image: `ghcr.io/rust-explorer/explorer:1.0.0` (`linux/amd64`).

### Features

- **Sign-in**: OIDC authorization code flow with PKCE on the server (tokens never reach the
  browser), access limited to one group, sliding sessions, sign-out at the provider. The
  provider may use a private CA (`EXPLORER_OIDC_CA_FILE`). Sign-in requests are rate limited
  per client address (per /64 for IPv6), taken from `X-Forwarded-For` when the peer is a
  trusted proxy: by default the loopback and private networks (`EXPLORER_TRUSTED_PROXIES`).
- **Browsing**: every folder mounted under `/data` is a volume, read-only when the container
  user cannot write to it. Sortable list and grid views with image thumbnails, favorite
  folders, search by name, refresh.
- **Downloads**: files with resumable `Range` requests, folders and multi-selections as
  streamed zips that list anything they had to leave out in `explorer-skipped.txt`.
- **Uploads**: files and folders, by button or drag and drop, in resumable 16 MiB chunks
  (tus) that survive a dropped connection or a reload, with Replace, Keep both or Cancel on
  a name conflict.
- **Managing files**: new folder, rename, and delete, copy and move, also between volumes,
  as background tasks with progress and cancellation.
- **Space usage**: what takes the room in a folder, as a navigable bar list.
- **Interface**: English, French and Spanish (also on the sign-in error pages), light and
  dark themes following the system, usable on a phone.
- **Security**: paths confined to their volume by the kernel (`openat2` with
  `RESOLVE_BENEATH`), strict CSP, downloads never rendered in the app's origin, cross-site
  requests refused. See [docs/security.md](docs/security.md).

### Requirements

- Docker on `linux/amd64`, a reverse proxy terminating TLS (the session cookie is `Secure`),
  and an OIDC provider that puts group names in a claim (tested with PocketID).
- The reverse proxy must accept and stream 16 MiB request bodies for uploads; see
  [docs/installation.md](docs/installation.md#reverse-proxy-and-uploads).

### Upgrading from a pre-release build

- Database migration 0004 adds a fingerprint to unfinished uploads: uploads started before
  the upgrade cannot be resumed and are uploaded again from the start.
- Sessions opened before the upgrade keep their fixed expiry until the next sign-in.

### Known limits

- Every member of the allowed group can read every volume and write to every writable one;
  there are no per-volume or per-folder permissions.
- Background tasks (copy, move, delete, space usage) live in memory: a restart loses them,
  and one interrupted that way may be half done.
- Volumes are discovered at startup; restart the container after adding a mount.
- Two uploads started at the same instant on two volumes of the same disk can both pass the
  free-space check.
- A sign-out in the instant before a session renewal is saved can be undone.
- A folder copy places files by path: after a rename swap during the copy, the remaining
  files can land in the folder that took the name (nothing is overwritten or deleted).
- A file edited in place within one modification-time tick without changing size is not seen
  as changed during a move between volumes.
