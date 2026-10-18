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
| `EXPLORER_OIDC_CLIENT_SECRET_FILE` | yes¹ | — | Path to a file containing the client secret (e.g. a Docker secret under `/run/secrets/`). Trailing newlines are ignored. |
| `EXPLORER_ALLOWED_GROUP` | yes | — | Only members of this group may sign in. |
| `EXPLORER_OIDC_GROUPS_CLAIM` | no | `groups` | ID token claim holding the user's group names. |
| `EXPLORER_SESSION_TTL` | no | `7d` | Idle session lifetime, e.g. `12h`, `7d`, `30d`. Must be greater than zero. |
| `EXPLORER_LOG` | no | `info` | Log filter: a level (`off`, `error`, `warn`, `info`, `debug`, `trace`) and/or comma-separated `target=level` directives, e.g. `warn,explorer_server=debug`. |
| `EXPLORER_LOG_FORMAT` | no | `pretty` | `pretty` (human-readable) or `json`. |

¹ Set exactly one of `EXPLORER_OIDC_CLIENT_SECRET` and `EXPLORER_OIDC_CLIENT_SECRET_FILE`.

OIDC sign-in is not implemented yet (milestone 2), but the OIDC variables are already
validated so a configuration written today keeps working.

## Paths and port

| Path | Purpose |
|---|---|
| `/data` | Each directory mounted under `/data` is a volume. |
| `/config` | Explorer's state. Must be writable by the container user. |
| `/usr/share/explorer/web` | Web bundle, shipped in the image. |

The server listens on `0.0.0.0:8080`. These are fixed in the image.

### Development-only overrides

For running the server outside Docker (development, tests), the paths can be overridden.
Do not set these in a container deployment.

| Variable | Default |
|---|---|
| `EXPLORER_DATA_DIR` | `/data` |
| `EXPLORER_CONFIG_DIR` | `/config` |
| `EXPLORER_WEB_DIR` | `/usr/share/explorer/web` |

## Container user

The process runs as the UID/GID from Docker's `user:` directive (image default
`1000:1000`). See [installation.md](installation.md#2-choose-the-uidgid).
