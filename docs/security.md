# Security

This page describes what Explorer protects against as of the sign-in milestone. To report
a vulnerability, follow [SECURITY.md](../SECURITY.md); do not open a public issue.

## Scope

Explorer sits behind a reverse proxy that terminates TLS and delegates identity to an OIDC
provider. It defends against: a malicious website acting on behalf of a signed-in user,
stolen or forged sign-in responses, token leakage to the browser, and users outside the
allowed group. It does not defend against a compromised provider, reverse proxy or host.
File access controls arrive with file management and are not covered here.

## Tokens stay on the server

Explorer is a backend-for-frontend. The server performs the OIDC authorization code flow
with PKCE (S256), `state` and `nonce`, and verifies the ID token (signature, issuer,
audience, expiry, nonce, access token hash). Access, ID and refresh tokens are never sent
to the browser and never logged. The browser only holds a session cookie.

## Session cookie

- Name `explorer_session`; `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
- Session data lives server-side in SQLite (`/config/explorer.db`); the cookie is an
  opaque identifier.
- Expiry is sliding: `EXPLORER_SESSION_TTL` of inactivity (default 7 days, capped at 400).
- The session id is rotated on successful login (against session fixation).
- Logout and access denial flush the session. After a denial only the ID token is kept,
  solely so that signing out can end the provider session.

Because the cookie is `Secure`, Explorer must be served over HTTPS.

## CSRF

`SameSite=Lax` is the first layer. In addition, every request other than `GET`, `HEAD` and
`OPTIONS` must carry an `Origin` (or, failing that, `Referer`) header whose origin equals
the origin of `EXPLORER_PUBLIC_URL`; otherwise the server answers 403 `csrf_rejected`.
Login is a harmless `GET` that only starts a sign-in; the callback is protected by the
`state` parameter, which is bound to the session and usable once.

## Open redirects

`/auth/login?return_to=` accepts only same-site absolute paths (a single leading `/`, no
`//` or `/\`, no control characters, at most 2048 bytes). Anything else becomes `/`. The
post-login redirect target is stored in the session, never taken from the callback.

## Groups gate

A user can sign in only if `EXPLORER_ALLOWED_GROUP` is in the groups claim
(`EXPLORER_OIDC_GROUPS_CLAIM`, ID token first, userinfo as fallback). A missing or
malformed claim fails the sign-in; it never grants access. Groups are read at sign-in
only; removing someone from the group takes effect when their session ends (log out, or
expiry), not immediately.

## Content Security Policy and headers

Every response carries a CSP of `default-src 'self'; script-src 'self' 'wasm-unsafe-eval';
style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none';
base-uri 'self'; form-action 'self'`, plus `X-Content-Type-Options` and `Referrer-Policy`.
Sign-in did not loosen it. The provider is reached by top-level redirects only.

## Secrets and logs

The client secret can be supplied as a file (`EXPLORER_OIDC_CLIENT_SECRET_FILE`) and is
never logged. Sign-in failures log a reason and the provider's error text (truncated);
rejected cross-origin requests log the offending origin (truncated). Tokens, codes,
session ids and the client secret are not logged. Provider requests do not follow
redirects, and discovery failures are retried. A discovery document that fails
validation (for example an issuer mismatch) stops startup; it is never used.
