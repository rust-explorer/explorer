# Explorer

Self-hosted web file manager written in Rust (Axum server, Dioxus web UI).

**Status: work in progress.** Sign-in with OIDC, browsing volumes and downloading files and
folders (as zip) work. Uploads and file operations are not implemented yet.

## Quick start

> You can browse and download but not change files yet, and no image is published yet; build one
> locally as described in [docs/development.md](docs/development.md#docker-image).

You need Docker, an OIDC provider such as [PocketID](https://pocket-id.org) and a reverse
proxy terminating TLS.

```sh
mkdir -p explorer/config && cd explorer
curl -fsSLo docker-compose.yml https://raw.githubusercontent.com/fabienlege/explorer/main/docker-compose.example.yml
curl -fsSLo .env https://raw.githubusercontent.com/fabienlege/explorer/main/.env.example
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
