# Explorer

Self-hosted web file manager written in Rust (Axum server, Dioxus web UI).

**Status: work in progress.** Sign-in with OIDC, browsing volumes, downloading files and
folders (as zip) and resumable uploads (files and folders, drag and drop, conflict handling)
work, and so does managing files: new folder, rename, multi-select (download several entries
as one zip), and delete, copy and move, including between volumes, as background tasks with
progress and cancellation. Tasks are not kept across a server restart. Also: refresh,
favorite folders, search by name, a grid view with image thumbnails, a space usage screen
(what takes the room in a folder) and dark mode.

## Quick start

> Images are published to `ghcr.io/rust-explorer/explorer` from the first release (v1.0.0) on.
> Before it, build one locally as described in
> [docs/development.md](docs/development.md#docker-image) and set `image:` to its tag.

You need Docker, an OIDC provider such as [PocketID](https://pocket-id.org) and a reverse
proxy terminating TLS.

```sh
mkdir -p explorer/config && cd explorer
curl -fsSLo docker-compose.yml https://raw.githubusercontent.com/rust-explorer/explorer/main/docker-compose.example.yml
curl -fsSLo .env https://raw.githubusercontent.com/rust-explorer/explorer/main/.env.example
# edit .env and docker-compose.yml, then:
docker compose up -d
```

See [docs/installation.md](docs/installation.md) for the full setup (OIDC client, UID/GID,
volumes, reverse proxy) and [docs/configuration.md](docs/configuration.md) for every
variable.

## License

Source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE). This is not OSI open source: commercial use is not permitted.

## Documentation

- [Installation](docs/installation.md)
- [Configuration](docs/configuration.md)
- [Architecture](docs/architecture.md)
- [Security](docs/security.md)
- [Development](docs/development.md)

Design and plans live in [`docs/`](docs/).
