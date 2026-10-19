# M2 Authentication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Only members of the configured OIDC group can use Explorer: unauthenticated users are sent to the provider (PocketID), the server holds the session, and the web app shows the signed-in user with a menu for language and sign-out.

**Architecture:** Backend-for-frontend OIDC in `crates/server` (`openidconnect` authorization code + PKCE + state + nonce). Server-side sessions in SQLite under `/config` via `tower-sessions`. Every state-changing request passes an Origin check. A `CurrentUser` extractor and an `authz::Policy` give every handler the user's identity, ready for multi-user later. The web app boots by calling `/api/me` and redirects to `/auth/login` on 401.

**Tech Stack:** Existing M1 stack, plus openidconnect 4.0 (feature `reqwest`, pulls reqwest 0.12 with rustls), tower-sessions 0.14, tower-sessions-sqlx-store 0.15 (feature `sqlite`), sqlx 0.8 (`runtime-tokio`, `sqlite`, `migrate`), time 0.3, dioxus `router` feature, gloo-net 0.7 (web), web-sys `Storage`/`Location`.
Version note: tower-sessions-sqlx-store 0.15 depends on tower-sessions-core 0.14, so tower-sessions must be **0.14**, and sqlx must be **0.8**.

**Spec:** `docs/superpowers/specs/2026-10-18-explorer-design.md` (milestone 2 of §14; §8 Security; §10 Configuration).

## Global Constraints

- Everything from the M1 plan's Global Constraints still applies: English, sparse comments, no commit rewriting, one stacked branch + PR per task (`m2/<slug>`, base = previous task branch, first one on `main`), owner merges, no personal data, `cargo xtask ci` passes before each PR, no UI control that does not work.
- Session cookie: name `explorer_session`, `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`; expiry `OnInactivity(config.session_ttl)`; session id rotated (`cycle_id`) on successful login.
- OIDC scopes: `openid profile email groups`. Groups read from the claim named `config.oidc.groups_claim` (default `groups`), which must be an array of strings. Access requires `config.oidc.allowed_group` in it.
- Redirect URI: `{public_url}auth/callback` (public_url is normalised with a trailing `/`).
- Database file: `{config_dir}/explorer.db`, opened with `mode=rwc`, WAL journal.
- JSON error codes (in `ErrorBody.code`): `unauthenticated` (401), `forbidden` (403), `csrf_rejected` (403), plus the existing `not_found`/`internal`.
- Sign-in failure reasons (query `reason` on `/signin-error`): `provider_error`, `state_mismatch`, `exchange_failed`, `invalid_token`, `missing_groups_claim`.
- Locale override persisted in `localStorage` key `explorer.locale`; values `en-US` | `fr-FR` | `es-ES`.
- No Dioxus `document::eval` (needs `new Function`, blocked by the CSP).
- No private keys committed, even test keys: generate test keys at test runtime.

## Review Focus

1. **Open redirect via `return_to`.** `/auth/login?return_to=//evil.example` or `/\evil.example` or `https://evil.example` must fall back to `/`. Test in Task 5.
2. **Replayed/forged callback.** A callback whose `state` doesn't match the pending login, or a second use of the same `state`, must fail with `state_mismatch` and must not create a session. Test in Task 5.
3. **PocketID restarting at the same time as Explorer** (compose start order). Discovery must retry instead of crash-looping instantly, then exit with one clear message if the issuer stays unreachable. Test in Task 4.
4. **Groups claim missing or wrong type.** The provider client lacks the `groups` scope, or the claim is a string rather than an array. The user must land on a clear error (`missing_groups_claim`), not a 500 or a misleading "access denied". Test in Task 5.
5. **Cross-site POST.** A form POST to `/auth/logout` from another origin, with the cookie attached because SameSite=Lax allows top-level navigations, must be rejected `csrf_rejected`. A request with neither `Origin` nor `Referer` must also be rejected. Test in Task 3.

---

### Task 1: M1 follow-ups and housekeeping

**Files:**
- Modify: `crates/server/src/config.rs`, `crates/server/Cargo.toml`, `crates/shared/Cargo.toml` (stray blank lines), `crates/server/tests/health.rs`, `crates/server/tests/healthcheck_cmd.rs`

**Interfaces:**
- Produces: `OidcConfig.issuer: String`, the issuer exactly as configured (validated as an http(s) URL, otherwise untouched). OIDC discovery compares it byte-for-byte with the provider's reported issuer, and `Url` would add a trailing `/`.

- [ ] **Step 1: Write failing tests**
  - In `config.rs`'s existing test module: `issuer_is_kept_verbatim`: `EXPLORER_OIDC_ISSUER=https://id.example.com` → `cfg.oidc.issuer == "https://id.example.com"`; with a trailing slash it stays with the slash.
  - `non_utf8_value_is_reported_invalid` (unix): `from_lookup` cannot express this, so add `Config::from_os_lookup(impl Fn(&str) -> Option<OsString>)` used by `from_env`; a non-UTF-8 `EXPLORER_PUBLIC_URL` gives `Invalid { var: "EXPLORER_PUBLIC_URL", reason contains "UTF-8" }`, not `Missing`. Keep `from_lookup` as the string-based wrapper used by tests.
  - `tests/health.rs`: `internal_error_is_generic_and_carries_request_id`. Mount a test-only route returning `ApiError::internal("db exploded")` on a router built from `router(state).merge(...)`. Expect 500, `code == "internal"`, a message that does not contain `exploded`, and `request_id` equal to the header.
  - `tests/healthcheck_cmd.rs`: `healthcheck_fails_on_non_200`. A listener answering `HTTP/1.1 503` → `Err`.
- [ ] **Step 2:** Run `cargo test -p explorer-server` → the new tests FAIL.
- [ ] **Step 3:** Implement the changes. Remove the stray blank lines in both Cargo manifests.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `chore(server): m1 review follow-ups`.

### Task 2: SQLite database and `/config` startup check

**Files:**
- Create: `crates/server/src/db.rs`, `crates/server/migrations/0001_init.sql` (`CREATE TABLE schema_info (id INTEGER PRIMARY KEY CHECK (id = 1), created_at TEXT NOT NULL);` plus its single row, so the migration pipeline is exercised from day one), `crates/server/tests/db.rs`
- Modify: `crates/server/src/app.rs` (`AppState` gains `db: SqlitePool`), `crates/server/src/main.rs`, `crates/server/tests/common/mod.rs`, workspace + server `Cargo.toml`

**Interfaces:**
- Consumes: `Config::config_dir`.
- Produces:
  ```rust
  pub async fn open(config_dir: &Path) -> Result<SqlitePool, DbError>; // creates dir file, WAL, runs sqlx::migrate!()
  #[derive(Debug, thiserror::Error)] pub enum DbError { NotWritable { dir: PathBuf, source: io::Error }, Open(#[from] sqlx::Error), Migrate(#[from] sqlx::migrate::MigrateError) }
  // AppState { config: Arc<Config>, db: SqlitePool, .. }
  ```
  `open` first checks writability by creating and deleting `{config_dir}/.write-test`. The `NotWritable` message names the directory, the effective UID/GID, and the hint "check the volume mount and the container `user:`".

- [ ] **Step 1: Write failing tests** in `tests/db.rs`:
  - `open_creates_database_and_runs_migrations`: tempdir → `explorer.db` exists and `SELECT COUNT(*) FROM schema_info` = 1.
  - `open_is_idempotent`: opening twice is ok.
  - `open_fails_clearly_on_read_only_dir` (unix): tempdir with mode 0o555 → `DbError::NotWritable` and a message containing the path. Skip if running as root (check `rustix::process::geteuid().is_root()`).
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. `main` calls `db::open` after config and before binding; on error it logs one line and exits with code 2. Test helpers build `AppState` with a pool in the test's tempdir.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS. Verify `docker build` still works (sqlx `migrate!` embeds the migrations at compile time, so `crates/server/migrations` must not be in `.dockerignore`).
- [ ] **Step 5:** Commit `feat(server): sqlite database with startup writability check`.

### Task 3: Sessions and CSRF protection

**Files:**
- Create: `crates/server/src/session.rs`, `crates/server/src/csrf.rs`, `crates/server/tests/csrf.rs`, `crates/server/tests/session.rs`
- Modify: `crates/server/src/app.rs`, `crates/server/src/main.rs`, `crates/server/src/error.rs` (new constructors), Cargo manifests

**Interfaces:**
- Consumes: `AppState.db`, `Config.session_ttl`, `Config.public_url`.
- Produces:
  ```rust
  // session.rs
  pub async fn layer(db: SqlitePool, ttl: Duration) -> Result<SessionManagerLayer<SqliteStore>, sqlx::Error>; // runs store.migrate()
  pub fn spawn_expired_cleanup(db: SqlitePool) -> tokio::task::JoinHandle<()>; // every 1 h
  // csrf.rs
  pub async fn require_same_origin(State(origin): State<Arc<str>>, req: Request, next: Next) -> Response;
  // error.rs
  impl ApiError { pub fn unauthenticated() -> Self; pub fn forbidden() -> Self; pub fn csrf_rejected() -> Self; }
  ```
  Exact behaviour of the CSRF check: for methods other than GET/HEAD/OPTIONS, the origin taken from `Origin` (or, if that is absent, from `Referer`) must equal `config.public_url.origin()`. Otherwise respond 403 `csrf_rejected` and log at `warn` with the request id and the offending origin.

- [ ] **Step 1: Write failing tests:**
  - `tests/csrf.rs` (router with a test-only `POST /api/_echo`):
    - `post_with_matching_origin_passes`
    - `post_with_foreign_origin_is_rejected` (`Origin: https://evil.example` → 403, `code == "csrf_rejected"`)
    - `post_with_matching_referer_and_no_origin_passes`
    - `post_without_origin_or_referer_is_rejected`
    - `get_is_never_checked`
    - `origin_null_is_rejected`
  - `tests/session.rs` (test-only route that writes then reads a session value):
    - `session_cookie_has_secure_attributes`: `Set-Cookie` contains `explorer_session=`, `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
    - `session_survives_store_roundtrip`: the second request with the cookie reads the value.
    - `unknown_session_cookie_is_ignored`
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement and wire it in `app.rs`: session layer inside the request-id/trace layers, CSRF middleware on the whole router. Test-only routes live in the test files, mounted via `router(state).merge(...)`. Do not add test routes to production code.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): sqlite-backed sessions and same-origin csrf check`.

### Task 4: OIDC client with discovery retry, and the fake provider test harness

**Files:**
- Create: `crates/server/src/oidc.rs`, `crates/fake-oidc/` (dev-only crate, `publish = false`: `src/lib.rs` with the fake provider, `src/main.rs` running it standalone for manual checks), `crates/server/tests/oidc_discovery.rs`
- Modify: `crates/server/src/main.rs`, `crates/server/src/app.rs` (`AppState` gains `oidc: Arc<Oidc>`), `crates/server/tests/common/mod.rs`, Cargo manifests (`fake-oidc` as a server dev-dependency; its deps: `rsa` for runtime test-key generation, `jsonwebtoken` or openidconnect's own signing types for minting ID tokens; builder's choice)

**Interfaces:**
- Consumes: `OidcConfig` (issuer is a verbatim `String`, Task 1), `Config.public_url`.
- Produces:
  ```rust
  pub struct Oidc { /* CoreClient + http client + end_session_endpoint: Option<Url> */ }
  impl Oidc {
      pub async fn discover(cfg: &OidcConfig, public_url: &Url) -> Result<Self, OidcError>;
      pub async fn discover_with_retry(cfg: &OidcConfig, public_url: &Url, attempts: u32, delay: Duration) -> Result<Self, OidcError>;
      pub fn authorize_url(&self) -> (Url, PendingLogin);
      pub async fn exchange(&self, code: String, pending: PendingLogin) -> Result<VerifiedIdentity, SigninError>;
      pub fn end_session_url(&self, id_token: &str, post_logout_redirect: &Url) -> Option<Url>;
  }
  #[derive(Serialize, Deserialize)] pub struct PendingLogin { pub csrf_state: String, pub nonce: String, pub pkce_verifier: String, pub return_to: String }
  pub struct VerifiedIdentity { pub sub: String, pub name: String, pub email: Option<String>, pub groups: Vec<String>, pub id_token: String }
  pub enum SigninError { ExchangeFailed(String), InvalidToken(String), MissingGroupsClaim }
  impl SigninError { pub fn reason(&self) -> &'static str; } // exchange_failed | invalid_token | missing_groups_claim
  ```
  - `name` resolves to the `name` claim, then `preferred_username`, then `sub`.
  - Groups are read from the ID token's additional claims under `groups_claim`. If they are absent there, read them from the userinfo endpoint when the provider advertises one. If they are still absent, or are not an array of strings, return `MissingGroupsClaim`.
  - The HTTP client does not follow redirects, as the openidconnect docs require for SSRF safety.
  - `main` calls `discover_with_retry(.., 10, 3s)`, logging each failed attempt at `warn` (`OIDC discovery failed (attempt 3/10): …`). The final failure is logged as one error line naming the issuer, and the process exits 2.

  Fake provider (`crates/fake-oidc`, a separate crate so tests and manual runs share it; never part of the image): an axum server on an ephemeral port serving:
  - `/.well-known/openid-configuration` with `issuer` equal to the server's base URL with no trailing slash;
  - `/jwks`, with an RSA key generated once per test binary via `OnceLock`;
  - `/token`, which checks `code`, `redirect_uri` and that the PKCE verifier matches the challenge recorded at `/authorize`, then returns an ID token signed RS256;
  - `/userinfo`;
  - `/end-session`.

  Each test controls the token through `FakeOidc::next_claims(serde_json::Value)`: groups, nonce override, expired `exp`, wrong `aud`. The harness also exposes `FakeOidc::approve(authorize_url) -> code`, which records the challenge, state and nonce exactly as a browser round trip would.

- [ ] **Step 1: Write failing tests** in `tests/oidc_discovery.rs`:
  - `discovers_fake_provider`
  - `issuer_mismatch_is_an_error`: configured issuer has a trailing slash, provider reports none → `Err`
  - `retry_succeeds_when_provider_comes_up_late`: bind the port, start serving after ~1 s, `attempts = 5, delay = 300ms` → `Ok`
  - `retry_gives_up_with_clear_error`: nothing listening, `attempts = 2, delay = 10ms` → `Err`, message names the issuer URL (Review Focus 3)
  - `authorize_url_contains_pkce_state_nonce_and_scopes`: query has `code_challenge_method=S256`, `state`, `nonce`, `scope` containing `openid profile email groups`, and `redirect_uri` = `{public_url}auth/callback`
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement `oidc.rs` and the harness, and wire discovery into `main`. Test helpers start a `FakeOidc` and point the config at it.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): oidc client with discovery retry`.

### Task 5: Auth routes, `CurrentUser`, `/api/me`, authz skeleton

**Files:**
- Create: `crates/server/src/auth.rs`, `crates/server/src/authz.rs`, `crates/server/tests/auth_flow.rs`
- Modify: `crates/server/src/app.rs`, `crates/shared/src/api.rs`

**Interfaces:**
- Consumes: `Oidc` (Task 4), sessions + CSRF (Task 3), `ApiError` constructors.
- Produces:
  ```rust
  // shared::api
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct Me { pub sub: String, pub name: String, pub email: Option<String> }
  #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct LogoutResponse { pub redirect_to: String }
  // server::auth
  #[derive(Clone, Debug, Serialize, Deserialize)] pub struct CurrentUser { pub sub: String, pub name: String, pub email: Option<String>, pub groups: Vec<String> }
  impl<S: Send + Sync> FromRequestParts<S> for CurrentUser // 401 ApiError::unauthenticated when no user in session
  pub fn routes() -> Router<AppState>; // /auth/login, /auth/callback, /auth/logout
  pub fn sanitize_return_to(raw: Option<&str>) -> String;
  // server::authz
  pub enum Action { Read, Write }
  pub struct Policy;
  impl Policy { pub fn check(&self, user: &CurrentUser, volume: &str, action: Action) -> Result<(), ApiError>; } // v1: always Ok
  ```
  Routes:
  - `GET /auth/login?return_to=`: stores `PendingLogin` under session key `oidc_pending` and responds 303 to the provider.
  - `GET /auth/callback?code&state` (or `error`): handles the provider's answer.
    - Success: the pending login is taken (removed) from the session, `state` is compared, the code is exchanged, and group membership is checked. Then `cycle_id`, `CurrentUser` stored under `user`, the id token stored under `id_token`, and a 303 to `return_to`.
    - Failure: 303 to `/signin-error?reason=<code>`, logged at `warn` with the request id.
    - Not in the group: 303 to `/access-denied` with no session user, logged at `info` with `sub` and the user's groups.
  - `POST /auth/logout`: flushes the session and returns `LogoutResponse`. `redirect_to` is the provider end-session URL when one is advertised (with `id_token_hint` and `post_logout_redirect_uri = public_url`), otherwise `/`.
  - `GET /api/me`: returns `Me` for a `CurrentUser`.

  `sanitize_return_to` accepts only values that start with `/`, whose second character is neither `/` nor `\`, that contain no control characters and that are at most 2048 bytes. Anything else gives `/`.

- [ ] **Step 1: Write failing tests** in `tests/auth_flow.rs`, all running against `FakeOidc`. A small cookie-jar helper carries `explorer_session` between requests.
  - `me_requires_login` → 401 `unauthenticated`
  - `full_login_flow_sets_session_and_redirects_to_return_to`: `/auth/login?return_to=/docs/a?b=1` → `FakeOidc::approve` → `/auth/callback` → 303 `Location: /docs/a?b=1`; then `/api/me` → `Me { name: "Ada Lovelace", .. }`
  - `session_id_rotates_on_login`: the cookie value differs before and after the callback
  - `user_outside_allowed_group_is_denied`: 303 `/access-denied`; `/api/me` still 401
  - `missing_groups_claim_reports_reason`, and the same with groups given as a string → `/signin-error?reason=missing_groups_claim` (Review Focus 4)
  - `state_mismatch_is_rejected` and `callback_replay_is_rejected`: a second callback with the same state → `state_mismatch`, no session user (Review Focus 2)
  - `expired_or_wrong_audience_token_is_invalid_token`
  - `provider_error_param_reports_provider_error`
  - `logout_clears_session_and_returns_end_session_url`, with a valid Origin; `/api/me` → 401 afterwards
  - `logout_without_origin_is_rejected`
  - `sanitize_return_to_rejects_open_redirects`: unit table test. `//evil.example`, `/\evil.example`, `https://evil.example`, `javascript:alert(1)`, `""`, a 3000-byte path and `/ok\n` all give `/`; `/docs/a?b=1` stays (Review Focus 1)
  - `authz` unit test: `Policy.check(..)` returns `Ok` for both actions
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement and mount the routes in `app.rs` (`/auth` routes keep the JSON 404 fallback for unknown paths).
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS.
- [ ] **Step 5:** Commit `feat(server): oidc login, callback, logout and current user`.

### Task 6: Web app — session boot, sign-in pages, user menu, language override

**Files:**
- Create: `crates/web/src/api.rs`, `crates/web/src/routes.rs`, `crates/web/src/components/mod.rs`, `crates/web/src/components/user_menu.rs`, `crates/web/src/pages/mod.rs`, `crates/web/src/pages/home.rs`, `crates/web/src/pages/access_denied.rs`, `crates/web/src/pages/signin_error.rs`
- Modify: `crates/web/src/app.rs`, `crates/web/src/i18n.rs`, `crates/web/locales/*.ftl`, `crates/web/Cargo.toml` (dioxus `router`, `gloo-net`, `explorer-shared`, web-sys features `Storage`, `Location`, `Document`, `Element`)

**Interfaces:**
- Consumes: `shared::api::{Me, LogoutResponse, ErrorBody}`; server routes from Task 5.
- Produces:
  ```rust
  // i18n.rs (host-testable)
  pub const STORAGE_KEY: &str = "explorer.locale";
  pub fn initial_locale(stored: Option<&str>, browser: &[String]) -> LanguageIdentifier; // valid stored value wins, else negotiate()
  pub fn native_name(lang: &LanguageIdentifier) -> &'static str; // "English" | "Français" | "Español"
  // api.rs (wasm)
  pub async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiFailure>; // credentials same-origin; on 401 → redirect to /auth/login?return_to=<current path+query>
  pub async fn post_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiFailure>;
  pub enum ApiFailure { Network, Server(ErrorBody) }
  ```
  Routes (Dioxus router):
  - `/access-denied`: translated message, plus a "Sign out" action that clears the provider session through `/auth/logout`, so the user can try another account.
  - `/signin-error?:reason`: a translated message per reason code and a "Try again" link to `/auth/login`.
  - Catch-all → `Home`, which calls `/api/me` and shows the M1 shell with a `UserMenu` (avatar with initials) in the header.

  `UserMenu`:
  - The dropdown shows the name, email if present, a language select (native names) and "Sign out".
  - It closes on outside click and on Escape.
  - Changing the language updates dioxus-i18n, `localStorage[explorer.locale]` and `<html lang>`, the latter via `web_sys::Document::document_element().set_attribute("lang", ..)`. `<html lang>` is also set at boot.

  Use the mockup's avatar style (indigo circle, initials) from `docs/design/mockup-main-screen.html`. All new strings go in all three `.ftl` files.

- [ ] **Step 1: Write failing tests** (host):
  - `initial_locale_prefers_valid_stored_value`: (`Some("es-ES")`, `["fr-FR"]`) → es-ES
  - `initial_locale_ignores_invalid_stored_value`: `Some("de-DE")` → negotiated
  - `native_names`
  - Existing locale-parity test keeps passing with the new keys.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. Browser-only code stays behind `cfg(target_arch = "wasm32")` like M1.
- [ ] **Step 4:** `cargo xtask ci --no-docker` → PASS, then `cargo clippy -p explorer-web --target wasm32-unknown-unknown -- -D warnings` → clean.
- [ ] **Step 5: Manual end-to-end check** against the fake provider. Run `cargo run -p fake-oidc` (the crate's binary, Task 4), which starts the fake provider on `127.0.0.1:8081` and auto-approves a demo user `Ada Lovelace` in group `explorer-users`. Add that auto-approve mode to the binary in this task if Task 4 didn't need it. Run the server with that issuer and the dx bundle, then in headless Chrome:
  - `/` redirects through the fake provider back to `/`, showing the avatar "AL";
  - the menu changes the language and `<html lang>`;
  - sign-out returns to the provider's end-session URL;
  - zero CSP violations.
  Record what was verified.
- [ ] **Step 6:** Commit `feat(web): session boot, sign-in pages and user menu`.

### Task 7: CI hardening and documentation

**Files:**
- Create: `docs/security.md`
- Modify: `xtask/src/main.rs`, `docs/installation.md`, `docs/configuration.md`, `docs/architecture.md`, `docs/development.md`, `README.md`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `cargo xtask ci` adds the step `cargo clippy -p explorer-web --target wasm32-unknown-unknown -- -D warnings` right after `dx build`.
  - When `CI` is set to `true` and `dx` is missing, the run fails instead of skipping.

- [ ] **Step 1:** Implement the xtask changes. Verify that `cargo xtask ci` passes and that `CI=true PATH=<without dx> cargo xtask ci --no-docker` fails with a clear message.
- [ ] **Step 2: Docs.**
  - `installation.md`: PocketID setup, checked against PocketID's current documentation and cited.
    - Create an OIDC client with callback URL `https://files.example.com/auth/callback` and logout callback `https://files.example.com`.
    - Create a group `explorer-users` and add your user to it.
    - Make sure groups are released to the client.
    - Fill in `.env`.
    - Troubleshooting: what each `/signin-error` reason means, and how to fix it.
  - `security.md`: the threat model for this milestone. Cover the BFF token handling, cookie attributes, CSRF Origin check, session rotation and expiry, open-redirect protection, groups gate, CSP, and secrets in logs. Explain how to report vulnerabilities (link `SECURITY.md`).
  - `README`: status now says sign-in works.
- [ ] **Step 3:** `cargo xtask ci` (with Docker) → PASS. actionlint is unchanged.
- [ ] **Step 4:** Commit `docs: sign-in setup, security model; ci: lint wasm target`.

---

## GitHub setup (once, before Task 1's PR)

Create one issue per task in milestone `M2 Authentication`, linking to this plan. Labels: Task 1 `backend`; Task 2 `backend`; Task 3 `backend,security`; Task 4 `backend,security`; Task 5 `backend,security`; Task 6 `frontend`; Task 7 `docs,ci`. Each PR body says `Closes #<issue>`.

## Manual acceptance by the owner (after merge)

Create the PocketID client per `docs/installation.md`, run the image, sign in with a passkey, check the avatar menu, sign out. Report any `/signin-error` reason seen.
