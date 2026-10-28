# Pre-1.0 Audit Fixes Implementation Plan

**Goal:** Fix every finding of the pre-1.0 audit (2026-10-28) before the v1.0 release and the opening of the repository.

**Architecture:** No new feature. Each fix lands in the domain that owns the code, with a test that reproduces the finding where one can be written. The audit reports (scenario, evidence and suggested fix per finding) were kept outside the repository; each work package below received its report.

**Spec:** `docs/superpowers/specs/2026-10-18-explorer-design.md`. Owner rulings (2026-10-28): fix everything; the owner's email in past merge commits stays (no history rewrite); the owner enables private vulnerability reporting; sessions become sliding.

## Global Constraints

- Everything from the M1–M6 plans' Global Constraints still applies.
- **Branch** `audit/fixes` from `main`, one commit per finding or per small group of findings of the same domain (`fix(<domain>): …`). Follow-up fixes are new commits.
- A behaviour or limit visible to users or operators is written in `docs/` (security, configuration) in the same commit.
- `cargo xtask ci` passes locally and on the Linux CI before merge.

## Work packages

| Package | Findings |
|---|---|
| Storage and tasks | S1 deep-tree delete overflows the stack · S2/T6 failed-copy cleanup removes another folder · T1 cross-volume move deletes changed sources · S3 staging guard race · S4 case-insensitive staging name · T2 permissions not kept by copy · T3 tree plan in memory, planning ignores cancel · T4 finished tasks unbounded · T5 "keep both" near 255 bytes |
| Uploads | U1 no limit on pending uploads · U2 slow PATCH without time limit · U3/T7 sweep vs staged copy · U4 sweep on a stale snapshot · U5 resume matching and kept conflict policy · U6 cancel during the last chunk |
| Auth and HTTP | A1 anonymous session rows · A2 sliding sessions · A3 two-tab sign-in · A4 ID token on logout · A5 same-site `/api` GETs · A6 JWKS refetch on unknown key · R3 database file permissions |
| Read side | R5-1 thumbnail decode memory · R5-2 zip skips odd names silently · R5-3/S5 zip and walks unbounded and not cancelled · R5-4 usage tree memory · R5-5 zip with unreadable parts · R5-6 search and Unicode normalisation · R5-7 thumbnail fds while queued · W1 download of a large selection |
| Web | W2 quadratic selection bookkeeping · W3 downloads navigate the tab · W4 new-folder dialog follows navigation · W5 rename of names with outer spaces · W6 busy dialogs · W7 non-virtualised lists · T8 finished task shown as failed |
| Release | R2 vulnerable TLS crates · R4 unverified build downloads, base images not pinned · R5 binstall version in CI · R6 stale docs · R7 advisories recorded, `cargo audit` in CI |
