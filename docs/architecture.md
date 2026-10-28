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

Status: `/healthz`, the web bundle and sign-in (`/auth/*`, `/api/me`) are implemented;
the rest of `/api/*` and `/files/*` are planned.

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
2. Routing: `/healthz`, then `/api/*` and `/auth/*` (JSON errors, never HTML), then the
   static bundle. Unknown non-asset paths fall back to `index.html` for client-side
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

Planned layering:

- `storage` — all filesystem access: volume discovery, path resolution and confinement,
  listing, file operations. Handlers never touch the filesystem directly.
- `uploads` — resumable uploads on top of `storage`.
- `api` — HTTP handlers mapping domain errors to the JSON error shape.
