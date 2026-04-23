# TODO

Active ship projects live in `.ship/NN-NAME/`. This file
is the light backlog — items not yet a ship project.

## Active

- See `.ship/06-PUBLISH/PROJECT.md`

## Backlog (not yet scoped)

- **Spec cleanup** — audits found bloat + stale + duplicated
  content in `specs/1/`. Research each finding against
  shipped code; trim what's in-code, capture what's not.
  Likely becomes `.ship/07-SPEC-CLEANUP/`.
- **Deployment** — public domain, Docker, TLS, one-click
  reviewer demo. Likely becomes `.ship/08-DEPLOY/`.
- **Signup/onboarding** — consumer auth flow, testnet
  balance seed, first-run tour. Likely becomes
  `.ship/09-SIGNUP/`.

## Conventions

- Project-level items with concrete acceptance criteria
  graduate to `.ship/NN-NAME/` via `/ship` skill.
- Per-session multi-step tracking uses `TaskCreate`, not
  this file.
- Architectural design questions go to `specs/`.
