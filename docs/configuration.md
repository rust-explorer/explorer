# Configuration

Explorer is configured through environment variables only. Startup validates all of them
and, if anything is wrong, exits with status 2 and prints one line per problem. Empty
values count as unset.

See [`.env.example`](../.env.example) for a starting point.

## Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `EXPLORER_PUBLIC_URL` | yes | — | External URL, e.g. `https://files.example.com`. Scheme, host and optional port only (no path, query or credentials). Used for the OIDC redirect URI and `Origin` checks. |
| `EXPLORER_OIDC_ISSUER` | yes | — | OIDC issuer URL, e.g. `https://id.example.com`. |
| `EXPLORER_OIDC_CLIENT_ID` | yes | — | OIDC client ID. |
| `EXPLORER_OIDC_CLIENT_SECRET` | yes¹ | — | OIDC client secret. |
| `EXPLORER_OIDC_CA_FILE` | no | — | Path to a PEM file of CA certificates the OIDC provider's certificate may be signed by. Trusted in addition to the built-in public roots, for discovery, key and token requests. Startup fails if the file is unreadable or has no certificate. |
| `EXPLORER_OIDC_CLIENT_SECRET_FILE` | yes¹ | — | Path to a file containing the client secret (e.g. a Docker secret under `/run/secrets/`). Trailing newlines are ignored. |
| `EXPLORER_ALLOWED_GROUP` | yes | — | Only members of this group may sign in. |
| `EXPLORER_OIDC_GROUPS_CLAIM` | no | `groups` | ID token claim holding the user's group names. |
| `EXPLORER_SESSION_TTL` | no | `7d` | Idle session lifetime, e.g. `12h`, `7d`, `30d`. Must be greater than zero; capped at 400 days. |
| `EXPLORER_TRUSTED_PROXIES` | no | — | Comma-separated networks (`172.18.0.0/16`) or addresses of the reverse proxies in front of Explorer; an IPv4-mapped network such as `::ffff:10.0.0.0/104` means its IPv4 network (`10.0.0.0/8`). A request from one of them is attributed to the client named in `X-Forwarded-For` (the rightmost entry that is not itself a trusted proxy); any other request to its TCP peer. Used by the sign-in rate limit. See [installation.md](installation.md#client-addresses-behind-the-proxy). |
| `EXPLORER_LOG` | no | `info` | Log filter: a level (`off`, `error`, `warn`, `info`, `debug`, `trace`) and/or comma-separated `target=level` directives, e.g. `warn,explorer_server=debug`. |
| `EXPLORER_LOG_FORMAT` | no | `pretty` | `pretty` (human-readable) or `json`. |

¹ Set exactly one of `EXPLORER_OIDC_CLIENT_SECRET` and `EXPLORER_OIDC_CLIENT_SECRET_FILE`.

Explorer requests the scopes `openid profile email groups`. The groups come from the
claim named by `EXPLORER_OIDC_GROUPS_CLAIM` in the ID token, falling back to the userinfo
endpoint; the claim must be an array of strings. Access requires `EXPLORER_ALLOWED_GROUP`
to be in it. Issuer discovery is retried 10 times, 3 seconds apart, at startup; only
configuration errors fail immediately.

`EXPLORER_SESSION_TTL` is a sliding window: activity renews the session, so it expires
after this much inactivity. To spare a database write per request, the renewal happens at
most once an hour (or every half TTL, if shorter). Values above 400 days are capped at 400
days. However active, a session ends 30 days after sign-in, or after `EXPLORER_SESSION_TTL`
if that is longer.

## Paths and port

| Path | Purpose |
|---|---|
| `/data` | Each real directory mounted under `/data` is a volume, named after its mount point. Read at startup. |
| `/config` | Explorer's state: sessions, upload records and favorites in `explorer.db` (SQLite), and the thumbnail cache. Must be writable by the container user; startup checks this and exits with a clear message otherwise. Explorer creates the database files with mode `600` and the cache folders with `700`, and sets those modes at startup on files and folders an earlier version left more open; keep `/config` itself `700` (startup logs a warning otherwise). |
| `/usr/share/explorer/web` | Web bundle, shipped in the image. |

The server listens on `0.0.0.0:8080`. These are fixed in the image.

### Development-only overrides

For running the server outside Docker (development, tests), the paths can be overridden.
Do not set these in a container deployment. `EXPLORER_DATA_DIR` is the directory scanned
for volumes.

| Variable | Default |
|---|---|
| `EXPLORER_DATA_DIR` | `/data` |
| `EXPLORER_CONFIG_DIR` | `/config` |
| `EXPLORER_WEB_DIR` | `/usr/share/explorer/web` |

## Thumbnail cache

Thumbnails of images are cached as files in `/config/cache/thumbnails`, created at startup
(a failure to create it stops the server). Nothing needs configuring. A thumbnail is
rendered on first request, which is why a large folder of photos fills the grid a few at a
time on the first visit. A cached file is touched when it is read after more than 7 days;
the server removes the files not used for 30 days at startup and then once a day, so the
cache stops growing with deleted or renamed images. It is only a cache: deleting the folder
is safe and the thumbnails are rebuilt on demand.

## Container user

The process runs as the UID/GID from Docker's `user:` directive (image default
`1000:1000`). See [installation.md](installation.md#2-choose-the-uidgid).
