# syntax=docker/dockerfile:1

# Base images are pinned by digest; bump the tag and the digest together.
ARG CARGO_CHEF_VERSION=0.1.78

FROM rust:1.93-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS chef
ARG CARGO_CHEF_VERSION
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup toolchain install
RUN cargo install --locked "cargo-chef@${CARGO_CHEF_VERSION}"

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# The bundle is platform-independent, so it is built natively. Trixie because the
# prebuilt dx binary needs glibc >= 2.39. No cargo-chef here: dx builds with its own
# injected profile, which a cooked cache would not match.
FROM --platform=$BUILDPLATFORM rust:1.93-trixie@sha256:ecbe59a8408895edd02d9ef422504b8501dd9fa1526de27a45b73406d734d659 AS web
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup toolchain install && rustup target add wasm32-unknown-unknown
# dx and the tools it would otherwise download at build time, each checked against a
# pinned SHA-256. wasm-bindgen must match the version in Cargo.lock; tailwindcss,
# binaryen and esbuild the versions dx 0.7.10 expects.
SHELL ["/bin/bash", "-euo", "pipefail", "-c"]
RUN case "$(uname -m)" in \
        x86_64) \
            dx=4363e4ed2a3f1eb7f4d38d2d59aed59ce43271c44c16b425e92c89a64761fbe7 \
            wb=82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e wb_target=x86_64-unknown-linux-musl \
            bn=50b9fa62b9abea752da92ec57e0c555fee578760cd237c40107957715d2976ba bn_arch=x86_64 \
            tw=9d258a7786c22f8572aea20ed35848f8cbef1d06277e4e6b97ad8561ffc21e07 tw_arch=x64 \
            eb=066e20cdb882994160e18524a552b97e03648eb9aa0c7cdf5680a6493be65ab2 eb_arch=x64 ;; \
        aarch64) \
            dx=8f1a17d3218700ffbe15e6540d936a178b2556fc801121a31082e3ba4ab9ef55 \
            wb=2ed4351c35dd9440308bbb02767d47ea278efe851a52465300f3c94f5b6c2a87 wb_target=aarch64-unknown-linux-musl \
            bn=81d46b86b10876ab615eec67e09fcc5615115a7b189cfe3d466725ee36c46ac2 bn_arch=aarch64 \
            tw=9555c6414617c15cc820679ec65404af92507e7b7718fd035d26df055561dec9 tw_arch=arm64 \
            eb=04f0bfb132b8b0800c23b22caa9ad7a7adf41e2434c027fc8571318b9904712f eb_arch=arm64 ;; \
        *) echo "unsupported build platform: $(uname -m)" >&2; exit 1 ;; \
    esac \
    && fetch() { curl -fsSL -o "$1" "$2" && echo "$3  $1" | sha256sum -c -; } \
    && bin=/usr/local/cargo/bin && tmp="$(mktemp -d)" && cd "$tmp" \
    && fetch dx.tgz "https://github.com/DioxusLabs/dioxus/releases/download/v0.7.10/dx-$(uname -m)-unknown-linux-gnu.tar.gz" "$dx" \
    && tar -xzf dx.tgz -C "$bin" dx \
    && fetch wb.tgz "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/0.2.129/wasm-bindgen-0.2.129-$wb_target.tar.gz" "$wb" \
    && tar -xzf wb.tgz -C "$bin" --strip-components=1 "wasm-bindgen-0.2.129-$wb_target/wasm-bindgen" \
    && fetch bn.tgz "https://github.com/WebAssembly/binaryen/releases/download/version_129/binaryen-version_129-$bn_arch-linux.tar.gz" "$bn" \
    && tar -xzf bn.tgz -C "$bin" --strip-components=2 binaryen-version_129/bin/wasm-opt \
    && fetch "$bin/tailwindcss" "https://github.com/tailwindlabs/tailwindcss/releases/download/v4.1.5/tailwindcss-linux-$tw_arch" "$tw" \
    && fetch eb.tgz "https://registry.npmjs.org/@esbuild/linux-$eb_arch/-/linux-$eb_arch-0.27.3.tgz" "$eb" \
    && tar -xzf eb.tgz -C "$bin" --strip-components=2 package/bin/esbuild \
    && chmod +x "$bin"/{dx,wasm-bindgen,wasm-opt,tailwindcss,esbuild} \
    && cd / && rm -rf "$tmp"
COPY . .
# NO_DOWNLOADS: dx takes its tools from PATH instead of fetching them.
RUN NO_DOWNLOADS=1 dx build --release -p explorer-web --platform web \
    && cp -r target/dx/explorer-web/release/web/public /out

FROM chef AS server
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p explorer-server --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p explorer-server \
    && mkdir -p /out/data /out/config \
    && chmod 700 /out/config \
    && cp target/release/explorer /out/explorer

FROM gcr.io/distroless/cc-debian12@sha256:e5d81ddde149641e2a9ba55be4545bc125c67de07508b03ba4c22e6eb0ded5aa
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
