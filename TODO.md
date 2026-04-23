# TODO

Active ship projects live in `.ship/NN-NAME/`. This file
is the light backlog — items not yet a ship project.

## Active

- `.ship/06-PUBLISH/PROJECT.md` — publish-readiness punch list
- `.ship/07-SPEC-CLEANUP/PROJECT.md` — spec cleanup (in progress)

## Scheduled (will become ship projects)

- **08-REST-ENDPOINTS** — gateway REST via CMP queries to risk
  (monoio, no Postgres in gateway). 5 endpoints + JWT + rate
  limits. Scoped.
- **09-DASHBOARDS** — finalize + SHIP all 5 dashboards
  (support, health, management, playground, risk). Simple,
  user-friendly. Scoped.
- **11-OAUTH** — GitHub OAuth via new Python `rsx-auth/`
  service. Users table, JWT issuance. Scoped.

## Backlog (not yet scoped)

- **10-DEPLOY** — public domain, Docker, TLS, one-click
  reviewer demo

## Conventions

- Project-level items with concrete acceptance criteria
  graduate to `.ship/NN-NAME/` via `/ship` skill.
- Per-session multi-step tracking uses `TaskCreate`, not
  this file.
- Architectural design questions go to `specs/`.
