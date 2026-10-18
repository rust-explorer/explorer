# M1 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A buildable, tested, containerised skeleton: an Axum server that validates its configuration, logs with request ids, exposes `/healthz`, and serves an empty-but-localised Dioxus web app, with local-first CI.

**Architecture:** Cargo workspace with `crates/server` (Axum binary), `crates/web` (Dioxus web app compiled to WASM by `dx`), `crates/shared` (types used by both), and `xtask` (the single CI entry point). The server serves the built web bundle as static files with an SPA fallback.

**Tech Stack:** Rust 1.93 (edition 2024), Tokio 1.53, Axum 0.8.9, tower 0.5, tower-http 0.7, tracing 0.1 / tracing-subscriber 0.3, secrecy 0.10, humantime 2.4, url 2.5, thiserror 2, Dioxus 0.7.10 (+ `dioxus-cli` 0.7.10), dioxus-i18n 0.6 (Fluent), unic-langid 0.9, Tailwind CSS (standalone CLI, invoked by `dx`).

**Spec:** `docs/superpowers/specs/2026-10-18-explorer-design.md` (milestone 1 of §14).

This is the first of seven milestone plans. Later milestones get their own plan, written when the previous one is merged.

## Global Constraints

- English for code, docs, commit messages, issues. Sparse comments; no tutorial-style comments. Proven patterns, no hacks.
- Never rewrite existing commits (no amend/squash/rebase); every change is a new commit. Merge `main` into a branch to update it.
- One branch + one PR per task; the owner merges. Branch name: `m1/<task-slug>`.
- No personal data anywhere: domains are `example.com`, volumes are `media`/`documents`, no real IPs/URLs/secrets.
- Commits use the repo-local identity already configured (GitHub noreply address).
- `rust-version = "1.93"`, `edition = "2024"` for every crate; versions pinned in `[workspace.dependencies]`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` must pass; run via `cargo xtask ci` before opening each PR (from Task 7 on; before that run the three commands directly).
- No UI control that does not work end to end.
- Env var names: `EXPLORER_PUBLIC_URL`, `EXPLORER_OIDC_ISSUER`, `EXPLORER_OIDC_CLIENT_ID`, `EXPLORER_OIDC_CLIENT_SECRET` (or `EXPLORER_OIDC_CLIENT_SECRET_FILE`), `EXPLORER_ALLOWED_GROUP`, `EXPLORER_OIDC_GROUPS_CLAIM` (default `groups`), `EXPLORER_SESSION_TTL` (default `7d`), `EXPLORER_LOG` (default `info`), `EXPLORER_LOG_FORMAT` (`pretty` default | `json`).
- Paths: `/data` (volumes), `/config` (state), web bundle at `/usr/share/explorer/web`. Overridable for dev/tests only via `EXPLORER_DATA_DIR`, `EXPLORER_CONFIG_DIR`, `EXPLORER_WEB_DIR` (Claude's call; documented as dev-only). Listen on `0.0.0.0:8080`.
- Locales: `en` (default, fallback), `fr`, `es`.
- Docker image: `linux/amd64` only, non-root default user `1000:1000`.

## Review Focus

1. Unknown `/api/...` or `/auth/...` paths must return a JSON 404 (`ErrorBody`), not the SPA `index.html` — otherwise later API typos look like successes. Test in Task 4.
2. The client secret must never appear in logs or `Debug` output of `Config`. Test in Task 2.
3. `EXPLORER_OIDC_CLIENT_SECRET_FILE` contents written by `echo` end with `\n`; trailing whitespace must be trimmed or OIDC will fail mysteriously. Test in Task 2.
4. `EXPLORER_PUBLIC_URL` given with a trailing slash or with a path/query must be normalised or rejected consistently (later used for redirect URI and Origin checks). Test in Task 2.
5. A missing/empty web bundle directory must fail at startup with one clear message, not serve 404s silently. Test in Task 4.

---

### Task 1: Workspace skeleton and repository hygiene

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`, `crates/shared/Cargo.toml`, `crates/shared/src/lib.rs`, `crates/server/Cargo.toml`, `crates/server/src/main.rs`, `crates/web/Cargo.toml`, `crates/web/src/main.rs`, `LICENSE`, `README.md`, `SECURITY.md`, `.env.example`, `.editorconfig`
- Modify: `.gitignore`

**Interfaces:**
- Produces: crate names `explorer-shared` (lib `shared`), `explorer-server` (bin `explorer`), `explorer-web` (bin, Dioxus). Workspace dependency table that later tasks extend.

- [ ] **Step 1:** Workspace `Cargo.toml` with `members = ["crates/*"]` (Task 7 adds `xtask`), `resolver = "3"`, `[workspace.package]` holding `edition`, `rust-version`, `license-file = "LICENSE"`, `repository = "https://github.com/fabienlege/explorer"`. `[workspace.lints.clippy]` with `pedantic = { level = "warn", priority = -1 }` plus `module_name_repetitions = "allow"`, `missing_errors_doc = "allow"`. Each crate uses `lints.workspace = true`.
- [ ] **Step 2:** `rust-toolchain.toml`: channel `1.93`, components `rustfmt`, `clippy`, targets `wasm32-unknown-unknown`.
- [ ] **Step 3:** Minimal crates: `shared` empty lib; `server` `main` printing nothing yet (`fn main() {}` is fine until Task 3); `web` with `dioxus = { version = "0.7.10", features = ["web"] }` rendering nothing yet.
- [ ] **Step 4:** `LICENSE`: verbatim text of PolyForm Noncommercial License 1.0.0 fetched from `https://polyformproject.org/licenses/noncommercial/1.0.0/` (plain-text version), with the line `Required Notice: Copyright Fabien LEGE (https://github.com/fabienlege)` at the top.
- [ ] **Step 5:** `README.md` (what Explorer is, status "work in progress", "source-available under PolyForm Noncommercial 1.0.0 — not OSI open source; commercial use is not permitted", link to `docs/`). `SECURITY.md` (report via GitHub private vulnerability reporting; do not open public issues). `.env.example` with every variable from Global Constraints, using `https://files.example.com`, `https://id.example.com`, `explorer-users`, `change-me`. `.gitignore` adds `/target`, `.env`, `docker-compose.override.yml`, `/crates/web/assets/tailwind.css` (generated), `.superpowers/` (already present).
- [ ] **Step 6:** Verify: `cargo build --workspace` succeeds and `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [ ] **Step 7:** Commit `chore: set up cargo workspace, license and repository files`.

### Task 2: Configuration loading and validation

**Files:**
- Create: `crates/server/src/config.rs`, `crates/server/src/lib.rs` (`pub mod config;`)
- Modify: `crates/server/Cargo.toml` (deps: `secrecy`, `humantime`, `url`, `thiserror`)

**Interfaces:**
- Produces:
  ```rust
  pub struct Config {
      pub public_url: Url,          // normalised: scheme+host[+port], no path, no trailing slash
      pub oidc: OidcConfig,
      pub session_ttl: Duration,
      pub log: LogConfig,
      pub data_dir: PathBuf,
      pub config_dir: PathBuf,
      pub web_dir: PathBuf,
  }
  pub struct OidcConfig { pub issuer: Url, pub client_id: String, pub client_secret: SecretString, pub allowed_group: String, pub groups_claim: String }
  pub struct LogConfig { pub filter: String, pub format: LogFormat }
  pub enum LogFormat { Pretty, Json }
  impl Config {
      pub fn from_env() -> Result<Self, ConfigErrors>;
      pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigErrors>;
  }
  #[derive(Debug, thiserror::Error)] pub enum ConfigError { Missing(&'static str), Invalid { var: &'static str, reason: String }, SecretFile { path: PathBuf, source: io::Error } }
  pub struct ConfigErrors(pub Vec<ConfigError>); // Display: one error per line
  ```
  `from_lookup` exists so tests never mutate the process environment (`set_var` is `unsafe` in edition 2024). `Config` derives `Debug` (secret shown as redacted by `secrecy`).

- [ ] **Step 1: Write failing tests** in `config.rs` `#[cfg(test)] mod tests`, with a helper `fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String>` and a `fn valid() -> Vec<(&str,&str)>` baseline:
  - `loads_valid_config_with_defaults`: groups_claim `"groups"`, session_ttl `Duration::from_secs(7*24*3600)`, log filter `"info"`, format `Pretty`, data_dir `/data`, config_dir `/config`, web_dir `/usr/share/explorer/web`.
  - `reports_all_missing_required_vars_at_once`: empty lookup → `ConfigErrors` containing `Missing` for exactly the 5 required vars (secret counts once).
  - `reads_secret_from_file_and_trims_trailing_newline`: tempfile containing `"s3cret\n"` via `EXPLORER_OIDC_CLIENT_SECRET_FILE` → `expose_secret() == "s3cret"`.
  - `rejects_both_secret_and_secret_file`: → `Invalid { var: "EXPLORER_OIDC_CLIENT_SECRET", .. }`.
  - `normalises_public_url_trailing_slash`: inputs `https://files.example.com` and `https://files.example.com/` both give `public_url.as_str() == "https://files.example.com/"`.
  - `rejects_public_url_with_path_or_query`: `https://example.com/explorer` and `https://example.com/?a=1` → `Invalid`. (Sub-path deployment is out of scope.)
  - `rejects_non_http_public_url`: `ftp://example.com` → `Invalid`.
  - `rejects_bad_session_ttl`: `"forever"` → `Invalid { var: "EXPLORER_SESSION_TTL", .. }`.
  - `rejects_bad_log_format`: `"xml"` → `Invalid`.
  - `debug_output_does_not_contain_secret`: `!format!("{cfg:?}").contains("s3cret")`.
  (`tempfile` as dev-dependency.)
- [ ] **Step 2:** Run `cargo test -p explorer-server config` → FAIL (types missing).
- [ ] **Step 3:** Implement `config.rs`. Collect every error instead of returning on the first; durations via `humantime::parse_duration`.
- [ ] **Step 4:** Run `cargo test -p explorer-server config` → PASS.
- [ ] **Step 5:** Commit `feat(server): load and validate configuration from environment`.

### Task 3: HTTP server core — logging, request ids, error body, health check

**Files:**
- Create: `crates/server/src/app.rs`, `crates/server/src/telemetry.rs`, `crates/server/src/error.rs`, `crates/shared/src/api.rs`, `crates/server/tests/health.rs`
- Modify: `crates/server/src/main.rs`, `crates/server/src/lib.rs`, `crates/shared/src/lib.rs`, Cargo manifests (deps: `tokio` [`rt-multi-thread`,`macros`,`signal`,`net`], `axum`, `tower`, `tower-http` [`trace`,`request-id`,`util`], `tracing`, `tracing-subscriber` [`env-filter`,`json`], `serde`, `rustix` [`process`]; dev: `http-body-util`, `serde_json`)

**Interfaces:**
- Consumes: `Config` (Task 2).
- Produces:
  ```rust
  // shared::api
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
  pub struct ErrorBody { pub code: String, pub message: String, #[serde(skip_serializing_if = "Option::is_none")] pub details: Option<serde_json::Value>, pub request_id: Option<String> }
  #[derive(Serialize, Deserialize)] pub struct Health { pub status: String, pub version: String }
  // server::error
  pub struct ApiError { status: StatusCode, code: &'static str, message: String }
  impl ApiError { pub fn not_found() -> Self; pub fn internal(err: impl std::fmt::Display) -> Self; }
  impl IntoResponse for ApiError // JSON ErrorBody; request_id filled by a middleware reading x-request-id
  // server::app
  #[derive(Clone)] pub struct AppState { pub config: Arc<Config> }
  pub fn router(state: AppState) -> Router;
  // server::telemetry
  pub fn init(log: &LogConfig);
  ```
  Request id: `SetRequestIdLayer::x_request_id(MakeRequestUuid)` + `PropagateRequestIdLayer`; `TraceLayer` span includes method, path, request id. `uuid` feature of tower-http or the `uuid` crate as needed.

- [ ] **Step 1: Write failing integration tests** in `tests/health.rs` using `router(state).oneshot(req)`:
  - `healthz_returns_ok_json`: GET `/healthz` → 200, body `Health { status: "ok", version: env!("CARGO_PKG_VERSION") }`.
  - `responses_carry_request_id`: any response has a non-empty `x-request-id` header.
  - `incoming_request_id_is_preserved`: request with `x-request-id: abc` → response header `abc`.
  - `unknown_api_route_returns_json_404`: GET `/api/nope` → 404, `content-type: application/json`, `ErrorBody.code == "not_found"`, `request_id == Some(<header value>)`.
  A test helper builds `Config` via `Config::from_lookup` with valid values and temp dirs.
- [ ] **Step 2:** Run `cargo test -p explorer-server --test health` → FAIL.
- [ ] **Step 3:** Implement `error.rs`, `app.rs` (`/healthz`, `/api` and `/auth` nested routers with JSON `not_found` fallback), `telemetry.rs` (pretty or JSON subscriber from `LogConfig`), and `main.rs`: load config (on error print `ConfigErrors` to stderr and exit code 2), init telemetry, log version and effective UID/GID (`rustix::process::getuid/getgid`), bind `0.0.0.0:8080`, `axum::serve(...).with_graceful_shutdown` on SIGTERM or Ctrl-C.
- [ ] **Step 4:** Run `cargo test -p explorer-server` → PASS.
- [ ] **Step 5:** Manual check: `EXPLORER_...=... cargo run -p explorer-server` with `.env.example` values, `curl -i localhost:8080/healthz` → 200 with `x-request-id`; running with no env prints the five missing-variable lines and exits 2.
- [ ] **Step 6:** Commit `feat(server): http core with request ids, json errors and health check`.

### Task 4: Serve the web bundle with SPA fallback and security headers

**Files:**
- Create: `crates/server/src/web.rs`, `crates/server/tests/static_files.rs`
- Modify: `crates/server/src/app.rs`, `crates/server/src/main.rs`, server `Cargo.toml` (tower-http `fs`, `set-header`)

**Interfaces:**
- Consumes: `AppState`, `Config::web_dir`, `ApiError`.
- Produces:
  ```rust
  pub fn check_bundle(web_dir: &Path) -> Result<(), BundleError>; // index.html must exist and be a file
  pub fn service(web_dir: &Path) -> Router; // ServeDir with fallback to index.html
  ```
  Security headers on every response: `content-security-policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'`, `x-content-type-options: nosniff`, `referrer-policy: same-origin`. Hashed assets (`/assets/*`) get `cache-control: public, max-age=31536000, immutable`; `index.html` gets `cache-control: no-cache`.

- [ ] **Step 1: Write failing tests** in `tests/static_files.rs` with a tempdir containing `index.html` (`<html>app</html>`) and `assets/app-abc.js`:
  - `serves_index_at_root`: GET `/` → 200, body contains `app`, `cache-control: no-cache`.
  - `serves_asset_with_immutable_cache`: GET `/assets/app-abc.js` → 200, `cache-control` contains `immutable`.
  - `unknown_client_route_falls_back_to_index`: GET `/media/Films` → 200 with index body.
  - `api_and_auth_routes_do_not_fall_back`: GET `/api/x` and `/auth/x` → 404 JSON `ErrorBody` (Review Focus 1).
  - `security_headers_present`: CSP contains `frame-ancestors 'none'`, plus `nosniff`.
  - `check_bundle_fails_on_missing_index`: empty tempdir → `Err`, message names the directory (Review Focus 5).
- [ ] **Step 2:** Run `cargo test -p explorer-server --test static_files` → FAIL.
- [ ] **Step 3:** Implement `web.rs`, mount it as the router fallback in `app.rs`, call `check_bundle` in `main.rs` before binding (on error: log one clear line, exit 2).
- [ ] **Step 4:** Run `cargo test -p explorer-server` → PASS.
- [ ] **Step 5:** Commit `feat(server): serve web bundle with spa fallback and security headers`.

### Task 5: Web app shell with Tailwind and i18n (en, fr, es)

**Files:**
- Create: `crates/web/Dioxus.toml`, `crates/web/src/i18n.rs`, `crates/web/src/app.rs`, `crates/web/locales/en-US.ftl`, `crates/web/locales/fr-FR.ftl`, `crates/web/locales/es-ES.ftl`, `crates/web/tailwind.css` (input), `crates/web/tests/locales.rs`
- Modify: `crates/web/src/main.rs`, `crates/web/Cargo.toml` (deps: `dioxus-i18n = "0.6"`, `unic-langid`, `web-sys` [`Navigator`,`Window`]; dev: `fluent-syntax`)

**Interfaces:**
- Produces:
  ```rust
  pub const SUPPORTED: [&str; 3] = ["en-US", "fr-FR", "es-ES"];
  pub fn negotiate(preferred: &[String]) -> LanguageIdentifier; // first match by language subtag, else en-US
  pub fn use_init_i18n_from_browser(); // reads navigator.languages, calls dioxus-i18n init
  ```
  Shell UI (`App`): full-height layout using the approved mockup's neutral palette, the product name `Explorer` and the translated tagline key `app-tagline`. Light/dark via Tailwind `dark:` variant driven by `prefers-color-scheme`. No buttons yet.
  Locale override from the avatar menu comes in M2 (the menu does not exist yet); `localStorage` key reserved: `explorer.locale`.

- [ ] **Step 1: Write failing tests**:
  - in `i18n.rs`: `negotiate_exact_match` (`["fr-FR"]` → fr-FR), `negotiate_language_only` (`["es-MX"]` → es-ES, `["fr"]` → fr-FR), `negotiate_first_supported_wins` (`["de-DE","fr-CA","en-GB"]` → fr-FR), `negotiate_falls_back_to_english` (`["de-DE"]` and `[]` → en-US).
  - in `tests/locales.rs`: `all_locales_define_the_same_message_ids` — parse every `.ftl` with `fluent-syntax`, compare the sets of message ids with `en-US.ftl`, fail listing missing/extra ids per locale; `no_locale_file_has_parse_errors`.
- [ ] **Step 2:** Run `cargo test -p explorer-web` → FAIL.
- [ ] **Step 3:** Implement `i18n.rs`, the `.ftl` files (keys: `app-name`, `app-tagline`, `loading`), `app.rs`, `main.rs` (`dioxus::launch(App)`), `Dioxus.toml` (app name `explorer`, Tailwind input `tailwind.css`). Tailwind v4 standalone CLI is what `dx` 0.7 drives automatically when `tailwind.css` is present; if this version requires explicit configuration, follow `dx` 0.7 docs and record the command in `docs/development.md`.
- [ ] **Step 4:** Run `cargo test -p explorer-web` → PASS.
- [ ] **Step 5:** Manual check: `cargo install dioxus-cli --version 0.7.10 --locked` (if absent), `dx build --release -p explorer-web --platform web`; locate the output `public/` dir (expected `target/dx/explorer-web/release/web/public`) and run the server with `EXPLORER_WEB_DIR` pointing at it; open `http://localhost:8080` in a browser set to French → French tagline, no console errors, CSP not violated (check devtools console).
- [ ] **Step 6:** Commit `feat(web): localised app shell with tailwind`.

### Task 6: Docker image, compose example and installation docs

**Files:**
- Create: `Dockerfile`, `.dockerignore`, `docker-compose.example.yml`, `docs/installation.md`, `docs/configuration.md`, `docs/architecture.md`, `docs/development.md`
- Modify: `README.md` (quick start section linking to installation)

**Interfaces:**
- Consumes: server binary `explorer`, web bundle output path confirmed in Task 5.
- Produces: image entrypoint `/usr/local/bin/explorer`, `EXPOSE 8080`, `USER 1000:1000`, `HEALTHCHECK` calling `explorer healthcheck` (a subcommand that GETs `http://127.0.0.1:8080/healthz` and exits 0/1 — keeps the runtime image free of curl). Volumes `/data`, `/config`.

- [ ] **Step 1: Write failing test** for the healthcheck subcommand: `crates/server/tests/healthcheck_cmd.rs` → `healthcheck_succeeds_against_running_server` (start `router` on an ephemeral port, call `explorer_server::healthcheck::run(addr)` → `Ok(())`) and `healthcheck_fails_when_nothing_listens` (`Err`). Signature: `pub async fn run(addr: SocketAddr) -> Result<(), HealthcheckError>`; the binary's `healthcheck` arg uses `127.0.0.1:8080`. Use a raw `tokio::net::TcpStream` HTTP/1.1 request, no HTTP client dependency.
- [ ] **Step 2:** Run → FAIL; implement `crates/server/src/healthcheck.rs` and the argv check in `main.rs`; run → PASS.
- [ ] **Step 3:** `Dockerfile` multi-stage: `rust:1.93-bookworm` builder with `cargo-chef` for dependency caching, installs `wasm32-unknown-unknown` and `dioxus-cli 0.7.10` (`cargo binstall` if available, else `cargo install --locked`), builds `dx build --release -p explorer-web` and `cargo build --release -p explorer-server`; runtime `gcr.io/distroless/cc-debian12:nonroot`-equivalent with UID 1000 (use `debian:bookworm-slim` + a created `explorer` user 1000:1000 if distroless UID mapping is awkward — builder's call), copies binary and web bundle to `/usr/share/explorer/web`.
- [ ] **Step 4:** `docker-compose.example.yml`: service `explorer`, image `ghcr.io/fabienlege/explorer:latest`, `user: "1000:1000"`, `env_file: .env`, volumes `./config:/config`, `/srv/media:/data/media`, `/srv/documents:/data/documents`, GoDoxy labels for host `files.example.com` → port 8080 (look up GoDoxy's current label syntax in its docs and cite the doc URL in `installation.md`).
- [ ] **Step 5:** Docs: `installation.md` (PocketID OIDC client: redirect URI `https://files.example.com/auth/callback`, scopes `openid profile email groups`, create group `explorer-users`; UID/GID choice with `id -u`; compose; first start), `configuration.md` (table of every variable from Global Constraints, defaults, dev-only overrides), `architecture.md` (crates, request flow, module layering from spec §6), `development.md` (toolchain, `dx`, running locally, `cargo xtask ci`).
- [ ] **Step 6:** Verify: `docker build -t explorer:dev .` succeeds; `docker run --rm -p 8080:8080 --env-file .env.example explorer:dev` → `curl localhost:8080/healthz` 200, browser shows the shell; `docker inspect` health becomes `healthy`; `docker run --rm explorer:dev` without env exits 2 listing missing variables.
- [ ] **Step 7:** Commit `feat: docker image, compose example and installation docs`.

### Task 7: Local-first CI (`cargo xtask ci`) and GitHub Actions

**Files:**
- Create: `xtask/Cargo.toml`, `xtask/src/main.rs`, `.cargo/config.toml` (alias `xtask = "run --package xtask --"`), `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- Modify: `Cargo.toml` (members add `xtask`), `docs/development.md`

**Interfaces:**
- Produces: `cargo xtask ci [--no-docker]` running in order, stopping at first failure and printing which step failed: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `dx build --release -p explorer-web` (skipped with a clear notice if `dx` is missing), `docker build -t explorer:ci .` (skipped with `--no-docker`). Uses `std::process::Command`, no extra deps.

- [ ] **Step 1:** Implement xtask; verify `cargo xtask ci --no-docker` passes locally and `cargo xtask ci` passes with Docker.
- [ ] **Step 2:** `ci.yml`: on `pull_request` and `push` to `main`, `paths-ignore: ['**.md', 'docs/**', 'LICENSE']`, `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`, `permissions: contents: read`, `ubuntu-latest`, toolchain from `rust-toolchain.toml`, `Swatinem/rust-cache@v2`, install `dx` via `cargo-binstall`, run `cargo xtask ci --no-docker`. Third-party actions pinned to full commit SHAs.
- [ ] **Step 3:** `release.yml`: on tags `v*` and `workflow_dispatch`; `permissions: contents: read, packages: write`; `docker/setup-buildx-action`, `docker/login-action` with `GITHUB_TOKEN`, `docker/build-push-action` with `platforms: linux/amd64`, tags `ghcr.io/fabienlege/explorer:<version>` and `:latest`, GHA cache. Pinned SHAs.
- [ ] **Step 4:** `docs/development.md`: CI is informative; required checks are not configured; minutes budget note (public repo → free; private → $0 limit stops jobs); never add a self-hosted runner (fork PRs would run on the NAS).
- [ ] **Step 5:** Verify workflows parse: `docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:latest` → no errors.
- [ ] **Step 6:** Commit `ci: local-first cargo xtask ci and github actions workflows`.

---

## GitHub setup (done once, before Task 1's PR)

Claude creates, on the owner's go: milestones `M1 Foundation` … `M7 v1.0 release` (titles from spec §14), labels `security`, `backend`, `frontend`, `docs`, `ci`, and one issue per task above in `M1 Foundation` linking to this plan. Each PR body says `Closes #<issue>`. Issues for M2–M7 are created when their plans are written.
