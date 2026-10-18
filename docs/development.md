# Development

## Toolchain

- Rust, pinned by [`rust-toolchain.toml`](../rust-toolchain.toml) (1.93, with `rustfmt`,
  `clippy` and the `wasm32-unknown-unknown` target). `rustup` installs it on first use.
- [Dioxus CLI](https://dioxuslabs.com/learn/0.7/) 0.7.10, which builds the web app:

  ```sh
  cargo install --locked dioxus-cli@0.7.10
  # or: cargo binstall dioxus-cli@0.7.10
  ```

  `dx` downloads Tailwind and `wasm-opt` on first build, so it needs network access.
- Docker, to build the image.

## Building the web app

```sh
dx build --release -p explorer-web --platform web
```

The bundle lands in `target/dx/explorer-web/release/web/public`. `dx` also generates
`crates/web/assets/tailwind.css` (git-ignored); a plain `cargo build` of the web crate
fails until `dx` has run once.

## Running locally

The server uses fixed container paths by default; override them with the
[development-only variables](configuration.md#development-only-overrides):

```sh
mkdir -p .dev/data/media .dev/config
set -a; . ./.env.example; set +a
EXPLORER_DATA_DIR=.dev/data \
EXPLORER_CONFIG_DIR=.dev/config \
EXPLORER_WEB_DIR=target/dx/explorer-web/release/web/public \
cargo run -p explorer-server
```

Then open <http://localhost:8080>. The server listens on port 8080; rebuild the web app
with `dx build` after frontend changes.

## Checks

These must pass before opening a pull request:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo xtask ci` will run the same checks plus the Docker build in a single command, and
CI will call it; until it lands, run the commands above.

## Docker image

```sh
docker build --platform linux/amd64 -t explorer:dev .
docker run --rm -p 8080:8080 --env-file .env.example explorer:dev
```

The image is built for `linux/amd64` only. On another architecture the build runs under
emulation and is slow.
