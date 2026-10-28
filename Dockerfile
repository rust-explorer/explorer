# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.93
ARG CARGO_BINSTALL_VERSION=1.26.0
ARG CARGO_CHEF_VERSION=0.1.78
ARG DIOXUS_CLI_VERSION=0.7.10

FROM rust:${RUST_VERSION}-bookworm AS chef
ARG CARGO_BINSTALL_VERSION
ARG CARGO_CHEF_VERSION
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup toolchain install
RUN curl -fsSL "https://github.com/cargo-bins/cargo-binstall/releases/download/v${CARGO_BINSTALL_VERSION}/cargo-binstall-$(uname -m)-unknown-linux-musl.tgz" \
        | tar -xz -C /usr/local/cargo/bin \
    && cargo binstall --no-confirm --locked "cargo-chef@${CARGO_CHEF_VERSION}"

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# The bundle is platform-independent, so it is built natively. Trixie because the
# prebuilt dx binary needs glibc >= 2.39. No cargo-chef here: dx builds with its own
# injected profile, which a cooked cache would not match.
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-trixie AS web
ARG CARGO_BINSTALL_VERSION
ARG DIOXUS_CLI_VERSION
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup toolchain install && rustup target add wasm32-unknown-unknown
RUN curl -fsSL "https://github.com/cargo-bins/cargo-binstall/releases/download/v${CARGO_BINSTALL_VERSION}/cargo-binstall-$(uname -m)-unknown-linux-musl.tgz" \
        | tar -xz -C /usr/local/cargo/bin \
    && cargo binstall --no-confirm --locked "dioxus-cli@${DIOXUS_CLI_VERSION}"
COPY . .
RUN dx build --release -p explorer-web --platform web \
    && cp -r target/dx/explorer-web/release/web/public /out

FROM chef AS server
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p explorer-server --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p explorer-server \
    && mkdir -p /out/data /out/config \
    && cp target/release/explorer /out/explorer

FROM gcr.io/distroless/cc-debian12
COPY --from=server /out/explorer /usr/local/bin/explorer
COPY --from=web /out /usr/share/explorer/web
COPY --from=server --chown=1000:1000 /out/data /data
COPY --from=server --chown=1000:1000 /out/config /config
USER 1000:1000
VOLUME ["/config"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
    CMD ["/usr/local/bin/explorer", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/explorer"]
