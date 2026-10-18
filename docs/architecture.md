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

Status: `/healthz` and the web bundle are implemented; `/api/*` and `/auth/*` currently
return JSON 404s; the rest is planned.

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

## Server modules

Implemented:

- `config` — loads and validates the environment at startup, reporting every problem.
- `app` — router, middleware stack, shared state.
- `web` — static bundle serving, cache headers, security headers.
- `error` — the JSON error type.
- `telemetry` — `tracing` setup (pretty or JSON).
- `healthcheck` — client used by `explorer healthcheck` for the Docker healthcheck.

Planned layering:

- `auth` — OIDC flow (authorization code + PKCE), sessions, `CurrentUser` extractor.
- `authz` — authorization policy. Every handler goes through it with the user, the volume
  and the action, so finer rules can be added later.
- `storage` — all filesystem access: volume discovery, path resolution and confinement,
  listing, file operations. Handlers never touch the filesystem directly.
- `uploads` — resumable uploads on top of `storage`.
- `api` — HTTP handlers mapping domain errors to the JSON error shape.
