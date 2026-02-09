# Local Agents Instructions (RSX)

This file declares available skills and where to read them. Do NOT inline
skill contents here. When a skill is used, read its `SKILL.md` at the path
listed below and follow those instructions.

## Skills (Claude Code Global Setup)

- cli: Command-line workflows and patterns
  Path: /home/onvos/.claude/skills/cli/SKILL.md
- commit: Commit discipline and workflow
  Path: /home/onvos/.claude/skills/commit/SKILL.md
- data: Data engineering patterns
  Path: /home/onvos/.claude/skills/data/SKILL.md
- go: Go development patterns
  Path: /home/onvos/.claude/skills/go/SKILL.md
- infrastructure: Infra/ops patterns
  Path: /home/onvos/.claude/skills/infrastructure/SKILL.md
- python: Python development patterns
  Path: /home/onvos/.claude/skills/python/SKILL.md
- refine: Refinement orchestration (delegates)
  Path: /home/onvos/.claude/skills/refine/SKILL.md
- rust: Rust development patterns
  Path: /home/onvos/.claude/skills/rust/SKILL.md
- service: Service patterns
  Path: /home/onvos/.claude/skills/service/SKILL.md
- ship: Plan-based delivery orchestration
  Path: /home/onvos/.claude/skills/ship/SKILL.md
- sql: SQL patterns
  Path: /home/onvos/.claude/skills/sql/SKILL.md
- trader: Trading systems patterns
  Path: /home/onvos/.claude/skills/trader/SKILL.md
- typescript: TypeScript development patterns
  Path: /home/onvos/.claude/skills/typescript/SKILL.md
- wisdom: General dev guidance
  Path: /home/onvos/.claude/skills/wisdom/SKILL.md

## Usage Rules

- If a task matches a skill, open and follow its `SKILL.md`.
- Do not copy full skill contents into prompts unless explicitly asked.
- If a skill file is missing or unreadable, say so and proceed with best
  fallback.
