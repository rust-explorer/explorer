# Installation

> **Status: work in progress.** Sign-in, browsing, downloads and uploads work; file
> operations (rename, move, delete) arrive in a later milestone.

Explorer ships as a single Docker image (`linux/amd64`). You need:

- Docker with Compose.
- An OIDC provider. [PocketID](https://pocket-id.org) is the documented and tested
  example; any compliant provider that can put group names in a claim works.
  The provider's TLS certificate must be publicly trusted (private CAs are not supported
  yet).
- A reverse proxy terminating TLS in front of the container. The example uses
  [GoDoxy](https://github.com/yusing/godoxy); any reverse proxy works.

The examples below use `files.example.com` for Explorer and `id.example.com` for PocketID.
Replace them with your own hostnames.

## 1. Create the OIDC client in PocketID

Menu names below follow PocketID's documentation at the time of writing; they may change
between versions.

1. Create a user group named `explorer-users` and add the people who may use Explorer
   (Administration → User Groups, see
   [User management](https://pocket-id.org/docs/setup/user-management)). Explorer matches
   the group's *name* (the one apps see in tokens), not its friendly name.
2. Create the client (Administration → OIDC Clients → Add OIDC Client, see
   [Connect an app](https://pocket-id.org/docs/setup/connect-an-app)):
   - Name: `Explorer`
   - Client type: confidential (not public), so it gets a client secret.
   - Callback URL: `https://files.example.com/auth/callback`
   - Logout callback URL: `https://files.example.com`
   - Copy the client ID and the client secret; the secret is shown only once.
3. On the client's Access tab, select the `explorer-users` group. A new client lets
   nobody in until you do
   ([Allowed user groups](https://pocket-id.org/docs/configuration/allowed-groups)).

Explorer requests the scopes `openid profile email groups`. PocketID releases the names
of the user's groups in the `groups` claim when the `groups` scope is requested
([Scopes and claims](https://pocket-id.org/docs/guides/scopes-and-claims)), so nothing
else is needed. To check what Explorer will receive, use the client's OIDC Data Preview.
If your provider uses another claim name, set `EXPLORER_OIDC_GROUPS_CLAIM`.

Register exactly `https://files.example.com` (no trailing slash) as the logout callback
URL: Explorer sends that string as the post-logout redirect, and the provider compares it
literally.

Other providers: the client needs the callback URL above and a groups claim. If the
provider has no end-session endpoint, signing out only ends the Explorer session.

## 2. Choose the UID/GID

Explorer runs as whatever user Docker's `user:` directive gives it (default `1000:1000`).
Files it creates belong to that user, and it can only read or write what that user can.
Use the account that owns your files on the host:

```sh
id -u   # UID
id -g   # GID
```

Explorer never changes ownership itself. At startup it logs the effective UID/GID.

The UID/GID decides what Explorer can see: a file this user cannot read is listed and
fails with `permission denied` when downloaded. A folder this user cannot search shows as
an unavailable item. A volume this user cannot write to is shown as read-only (see
step 4).

## 3. Prepare the directory

```sh
mkdir explorer && cd explorer
mkdir -m 700 config
sudo chown 1000:1000 config   # use your UID:GID
```

`config/` holds Explorer's state (database, thumbnail cache). Create it yourself before
the first start: if Docker creates a missing bind-mount source, it is owned by root and
Explorer cannot write to it. Keep it `700`: the database holds live session ids, and
anyone who can read it can sign in as those users. Explorer creates `explorer.db` (and its
`-wal` and `-shm` files) readable by its own user only, and tightens them at startup if an
older version left them readable by others.

Download [`docker-compose.example.yml`](../docker-compose.example.yml) as
`docker-compose.yml` and [`.env.example`](../.env.example) as `.env`.

## 4. Configure

Edit `.env`:

```sh
EXPLORER_PUBLIC_URL=https://files.example.com
EXPLORER_OIDC_ISSUER=https://id.example.com
EXPLORER_OIDC_CLIENT_ID=<client ID from step 1>
EXPLORER_OIDC_CLIENT_SECRET=<client secret from step 1>
EXPLORER_ALLOWED_GROUP=explorer-users
```

Every variable is described in [configuration.md](configuration.md). To keep the secret out of
`.env`, use `EXPLORER_OIDC_CLIENT_SECRET_FILE` with a Docker secret instead.

Edit `docker-compose.yml`:

- `user:` — the UID:GID from step 2.
- `volumes:` — every directory mounted under `/data` becomes a volume in Explorer, named
  after its mount point. Only mounted directories are reachable. For example, to mount
  `/srv/media` as `media` and `/srv/documents` as read-only `documents`:

  ```yaml
  volumes:
    - ./config:/config
    - /srv/media:/data/media
    - /srv/documents:/data/documents:ro
  ```

  A volume is shown as read-only in Explorer when the container user cannot write to its
  root: because of a `:ro` mount, or because of the folder's owner and permissions. Volumes
  are discovered at startup, so restart the container after adding a mount. Symlinks
  directly under `/data` are ignored, and symlinks inside a volume are followed only when
  they are relative and stay inside it (see [security.md](security.md#symlinks)).
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
  container's port 8080. Uploads need a few more things from the proxy, see
  [Reverse proxy and uploads](#reverse-proxy-and-uploads).

## 5. First start

```sh
docker compose up -d
docker compose logs -f explorer
```

On success the log shows the version, the effective UID/GID, one `volume found` line per
volume (with `read_only=true` or `false`) and `listening` on `0.0.0.0:8080`. At startup
Explorer contacts the issuer; if it is unreachable it retries 10 times, 3 seconds apart
(so Explorer and PocketID can start together), then exits.
If the configuration is incomplete or invalid, the container exits with status 2 and
prints one line per problem, for example:

```
EXPLORER_PUBLIC_URL is required
EXPLORER_OIDC_ISSUER is required
```

Two warnings are worth knowing:

- `volume media is not writable by uid 1000 gid 1000; check the mount and the container
  user:` — the volume is shown as read-only. Expected for a `:ro` mount; otherwise fix the
  ownership of the host folder or the `user:` directive.
- `no volumes found under /data; mount folders as /data/<name>` — nothing is mounted
  under `/data`, so Explorer has nothing to show. If `/data` itself cannot be read, startup
  fails with status 2 and `data directory unreadable`.

The image has a built-in healthcheck (`explorer healthcheck`, which queries
`/healthz`); `docker compose ps` shows the container as `healthy` once it is up.

Then open `https://files.example.com`: you are sent to PocketID, and back once signed in,
then to your first volume. Click a file to download it, or a folder's download button to
get it as a zip. Use the Upload button (or the "+" on a phone), or drop files and folders
on the listing, to upload.
Explorer's session lives in `/config/explorer.db`, so `config/` must be writable by the
container user; otherwise startup fails with a message naming the directory and the
UID/GID.

## Reverse proxy and uploads

Explorer sends each file as a series of 16 MiB chunks, one `PATCH /files/uploads/<id>` per
chunk, and an interrupted transfer resumes where the server stopped. The proxy in front of
it must therefore:

- accept request bodies of at least 16 MiB (many proxies default to 1 MiB or less; when
  there is a configurable limit, set it to 16 MiB or more, or remove it);
- not buffer request bodies: stream them to Explorer as they arrive. A proxy that stores a
  whole chunk before forwarding it (to memory or to disk) delays and multiplies the work and
  can break the idle detection below;
- allow a few minutes per request (read, send and idle timeouts). A chunk on a slow uplink
  takes a while: 16 MiB at 1 MB/s is about 16 s, at 100 kB/s nearly three minutes. A request
  that is cut off is not lost, as the client resumes from the server's offset, but a timeout
  shorter than a chunk makes a slow connection retry forever.

Explorer itself closes an upload request whose body sends nothing for 60 seconds, and a zip
download whose client stops reading for 60 seconds; a proxy should not impose a shorter
idle limit than that. The exact setting names depend on your proxy; check its documentation
for the maximum request body size, request buffering and read/send timeouts. The GoDoxy
labels above do not set any of these, and this guide does not list GoDoxy-specific values.

Also keep in mind:

- Each writable volume gets a `.explorer-uploads` folder at its root, created by the first
  upload (mode `0700`, owned by the container user). Incoming data is staged there and
  moved into place when complete, so a volume needs room for the files being uploaded, and
  the folder is hidden from listings and zips. Abandoned uploads are removed after 24 hours.
  Do not delete or rename it while uploads are running.
- Uploads and copies are staged inside the volume, and a move within a volume is a rename, so
  the final step must stay on one filesystem. If a folder inside a volume is a separate mount
  (another disk or dataset), uploads, copies and moves into it fail with `cross_device` ("on
  another filesystem than its volume") and the log says the destination is on another
  filesystem. Mount that folder as its own volume under `/data` instead.
- A read-only volume (`:ro` mount, or not writable by the container user) cannot receive
  uploads; its upload controls are disabled and the server answers `read_only_volume`.
- Uploaded files belong to the container user and get the container's umask, like any file
  it creates (`0644` with the usual `022`). To keep them private from other host users,
  restrict access to the volume's folder on the host.
- The folder picker cannot see empty folders, because browsers only list files; drag and
  drop does create them.

## Troubleshooting sign-in

A failed sign-in lands on a page showing `/signin-error?reason=...`. The server log has
the details (look for `sign-in failed` with the same `reason`).

| Reason | Meaning | Fix |
|---|---|---|
| `provider_error` | The provider returned an error instead of a code. | Read the log line's `error`. Typical causes: the user cancelled, or the provider refused this user or client (`access_denied`). |
| `state_mismatch` | The sign-in was expired, replayed, forged, or belongs to another browser tab. | Retry from the start. Check cookies are not blocked and `EXPLORER_PUBLIC_URL` is the URL in the address bar (the session cookie is `Secure`, so HTTPS is required). |
| `exchange_failed` | The token endpoint rejected the code, or could not be reached. | Check the client ID and secret, that the client is confidential, and that the container can reach the issuer. |
| `invalid_token` | The ID token failed verification (signature, issuer, audience, expiry or nonce). | Check `EXPLORER_OIDC_ISSUER` matches the provider's issuer exactly, the client ID, and the clocks of both hosts. After a rotation of the provider's signing keys, Explorer fetches them again by itself, at most once a minute. |
| Signing out stops on PocketID's own sign-out page | The logout callback URL does not match exactly. | Register `https://files.example.com` with no trailing slash (step 1). |
| `missing_groups_claim` | The groups claim is absent or not an array of strings. | Make sure the `groups` scope is released to the client and, for another provider, set `EXPLORER_OIDC_GROUPS_CLAIM`. |

Signing in successfully but landing on `/access-denied` means the user is not in
`EXPLORER_ALLOWED_GROUP`; the log line `access denied` lists the groups received.

## Troubleshooting downloads

A download that fails after it started cannot show an error page: the browser lists it as
failed in its download list. The reason is in the server log (`zip aborted` names the
path that failed; storage failures show the code, the volume and the path; only
unexpected I/O failures also show the OS error). A large download interrupted by the
network can be resumed by the browser, as files support `Range` requests.

If a download is cut at a fixed time or size, check the reverse proxy's timeouts and
buffering limits. A zip download is also aborted when the client stops reading for 60
seconds. `/files/zip` requires the `path` parameter; an empty `path` downloads the whole
volume.

## Updating

```sh
docker compose pull
docker compose up -d
```
