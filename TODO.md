# TODO

Active ship projects live in `.ship/NN-NAME/`. This file
is the light backlog — items not yet a ship project.

## Active

- `.ship/06-PUBLISH/PROJECT.md` — publish-readiness punch list
- `.ship/07-SPEC-CLEANUP/PROJECT.md` — spec cleanup (in progress)

## Scheduled (will become ship projects)

- **08-REST-ENDPOINTS** — FULL impl of gateway REST (5
  endpoints, JWT, rate limits, CORS, tests, integration
  tests). Scoped after 07 completes.
- **09-DASHBOARDS** — finalize + SHIP all 5 dashboards
  (support, health, management, playground, risk). Simple,
  user-friendly. Scoped after 07 completes.

## Backlog (not yet scoped)

- **10-DEPLOY** — public domain, Docker, TLS, one-click
  reviewer demo
- **11-SIGNUP** — consumer auth flow, testnet balance seed,
  first-run tour

## Conventions

- Project-level items with concrete acceptance criteria
  graduate to `.ship/NN-NAME/` via `/ship` skill.
- Per-session multi-step tracking uses `TaskCreate`, not
  this file.
- Architectural design questions go to `specs/`.
