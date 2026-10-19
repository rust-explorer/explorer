# Architecture

Explorer is one Docker container: an Axum server that serves a JSON API and a Dioxus web
app compiled to WebAssembly. A reverse proxy terminates TLS in front of it.

```
browser ──HTTPS──> reverse proxy ──HTTP──> explorer container
                                            ├─ Axum server (:8080)
                                            │   ├─ /healthz    health check
                                            │   ├─ /api/*      JSON API
                                            │   ├─ /auth/*     OIDC login/callback/logout
                                            │   ├─ /files/*    upload / download / zip
                                            │   └─ /*          web bundle (WASM), SPA fallback
                                            ├─ /data/<volume>  mounted volumes
                                            └─ /config         state (SQLite, thumbnail cache)
```

Status: `/healthz`, the web bundle, sign-in (`/auth/*`, `/api/me`), browsing
(`/api/volumes*`) and downloads (`/files/download`, `/files/zip`) are implemented; uploads
and file operations are planned.

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
| `GET /files/zip?volume=&path=` | A folder (or the whole volume when `path` is empty) as a zip, streamed. |

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
| `range_not_satisfiable` | 416 | The requested range cannot be satisfied (it starts past the end of the file). |

Any other I/O failure is a 500 `internal` whose cause is only in the server log.

Downloads: the body is streamed from the open file, so memory use does not depend on the
file size. The zip is streamed without compression (`stored`) with zip64 entries, so
there is no size limit and no temporary file. The folder is walked before the first byte is
sent, so a missing or unreadable folder is a normal JSON error. A failure after streaming
has started (for example a file that becomes unreadable) cannot change the status code:
the server aborts the body, the browser reports a failed download, and the reason is in
the server log (`zip aborted`, with the path).

## Web app

Routes: `/` (opens the first volume, or says there is none), `/browse/<volume>/<path>`,
`/access-denied`, `/signin-error` and a not-found page. The folder lives in the URL, so
links, reload and the back button work. The sidebar lists the volumes with their free
space and a read-only marker. The folder is shown as a table sorted by name, size or date
(folders first), rendered through a virtualised list so a folder with tens of thousands of
entries keeps the DOM small. Sizes use SI units (1 kB = 1000 B). Each file row links to
`/files/download` and each folder row to `/files/zip`; these are plain links, so the
browser handles the download and shows any failure in its own download list.

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
  - `error` — storage errors, mapped to API error codes in `error`.
- `api` — `GET /api/volumes` and `GET /api/volumes/{volume}/entries`.
- `files` — `/files/download` and `/files/zip`.

Planned:

- `uploads` — resumable uploads on top of `storage`.
