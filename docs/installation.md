# Installation

> **Status: work in progress.** Explorer currently serves only the web shell. Sign-in
> (OIDC) arrives in milestone 2, file management after it. The server already requires
> the OIDC settings below and refuses to start without them, so configure them now and
> the setup carries over unchanged. This page describes the target setup.

Explorer ships as a single Docker image (`linux/amd64`). You need:

- Docker with Compose.
- An OIDC provider. [PocketID](https://pocket-id.org) is the documented and tested
  example; any compliant provider that can put group names in a claim works.
- A reverse proxy terminating TLS in front of the container. The example uses
  [GoDoxy](https://github.com/yusing/godoxy); any reverse proxy works.

The examples below use `files.example.com` for Explorer and `id.example.com` for PocketID.
Replace them with your own hostnames.

## 1. Create the OIDC client in PocketID

1. In PocketID's admin area, create a user group named `explorer-users` and add the
   people who may use Explorer. Only members of this group can sign in.
2. Create an OIDC client:
   - Name: `Explorer`
   - Callback URL: `https://files.example.com/auth/callback`
   - Keep it a confidential (non-public) client so it gets a client secret.
3. Note the client ID and client secret.

Explorer requests the scopes `openid profile email groups`. PocketID adds the user's
group names to the `groups` claim when the `groups` scope is requested. If your provider
uses another claim name, set `EXPLORER_OIDC_GROUPS_CLAIM`.

## 2. Choose the UID/GID

Explorer runs as whatever user Docker's `user:` directive gives it (default `1000:1000`).
Files it creates belong to that user, and it can only read or write what that user can.
Use the account that owns your files on the host:

```sh
id -u   # UID
id -g   # GID
```

Explorer never changes ownership itself. At startup it logs the effective UID/GID.

## 3. Prepare the directory

```sh
mkdir explorer && cd explorer
mkdir config
sudo chown 1000:1000 config   # use your UID:GID
```

`config/` holds Explorer's state (database, thumbnail cache). Create it yourself before
the first start: if Docker creates a missing bind-mount source, it is owned by root and
Explorer cannot write to it.

Download [`docker-compose.example.yml`](../docker-compose.example.yml) as
`docker-compose.yml` and [`.env.example`](../.env.example) as `.env`.

## 4. Configure

Edit `.env` with your public URL, issuer, client ID, client secret and group. Every
variable is described in [configuration.md](configuration.md). To keep the secret out of
`.env`, use `EXPLORER_OIDC_CLIENT_SECRET_FILE` with a Docker secret instead.

Edit `docker-compose.yml`:

- `user:` — the UID:GID from step 2.
- `volumes:` — every directory mounted under `/data` becomes a volume in Explorer, named
  after its mount point. Only mounted directories are reachable. The example mounts
  `/srv/media` as `media` and `/srv/documents` as `documents`. Mount a volume read-only
  (`:ro`) to make it read-only in Explorer.
- `labels:` — GoDoxy route for your hostname, pointing at port 8080:

  ```yaml
  labels:
    proxy.aliases: files.example.com
    proxy.#1.port: "8080"
    proxy.#1.healthcheck.path: /healthz
  ```

  Label syntax: [GoDoxy — Configuring Routes](https://docs.godoxy.dev/docs/godoxy/getting-started/Configuring-Routes).
  GoDoxy must be able to reach the container (same Docker host/network). With another
  reverse proxy, remove the labels and proxy `https://files.example.com` to the
  container's port 8080.

## 5. First start

```sh
docker compose up -d
docker compose logs -f explorer
```

On success the log shows the version, the effective UID/GID and `listening` on
`0.0.0.0:8080`. If the configuration is incomplete or invalid, the container exits with
status 2 and prints one line per problem, for example:

```
EXPLORER_PUBLIC_URL is required
EXPLORER_OIDC_ISSUER is required
```

The image has a built-in healthcheck (`explorer healthcheck`, which queries
`/healthz`); `docker compose ps` shows the container as `healthy` once it is up.

Then open `https://files.example.com`.

## Updating

```sh
docker compose pull
docker compose up -d
```
