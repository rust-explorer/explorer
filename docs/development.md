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

### Identity provider

Sign in through a real PocketID (or other OIDC provider) instance. Register a client as
described in [installation](installation.md), adding `http://localhost:8080/auth/callback`
as a callback URL and `http://localhost:8080` as a logout callback URL, then run the server
with your values:

```sh
set -a; . ./.env.example; set +a
EXPLORER_PUBLIC_URL=http://localhost:8080 \
EXPLORER_OIDC_ISSUER=https://id.example.com \
EXPLORER_OIDC_CLIENT_ID=... \
EXPLORER_OIDC_CLIENT_SECRET=... \
EXPLORER_DATA_DIR=.dev/data \
EXPLORER_CONFIG_DIR=.dev/config \
EXPLORER_WEB_DIR=target/dx/explorer-web/release/web/public \
cargo run -p explorer-server
```

The session cookie is `Secure`; Chrome and Firefox accept it on `localhost` over plain HTTP;
other browsers may not. The integration tests use an in-process provider
(`crates/server/tests/common/fake_oidc.rs`) and need no network.

## Checks

These must pass before opening a pull request:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Or run everything in one command:

```sh
cargo xtask ci              # the checks above, `dx build --release`, clippy for wasm32, then `docker build -t explorer:ci .`
cargo xtask ci --no-docker  # skip the Docker build
```

Steps run in order and stop at the first failure. The `dx build` and wasm clippy steps are
skipped with a notice if `dx` is not installed, except when `CI=true`, where a missing
`dx` fails the run. `dx` must run before any clippy or build of the web crate for the
`wasm32` target, because it generates `tailwind.css`; the host-target checks above do not
need it.

## CI

GitHub Actions run the same checks (`cargo xtask ci --no-docker`) on pull requests and
pushes to `main`, and `release.yml` publishes `ghcr.io/fabienlege/explorer` on `v*` tags.

- CI is informative. No required status checks are configured; run `cargo xtask ci`
  locally before opening a pull request.
- Minutes budget: Actions are free for a public repository. While the repository is
  private, the free quota applies and a $0 spending limit stops jobs once it is used up.
- Never add a self-hosted runner. Pull requests from forks would execute their code on
  that machine.

## Docker image

```sh
docker build --platform linux/amd64 -t explorer:dev .
docker run --rm -p 8080:8080 --env-file .env.example explorer:dev
```

The image is built for `linux/amd64` only. On another architecture the build runs under
emulation and is slow.
