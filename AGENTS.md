# Repository Guidelines

## Project Structure & Module Organization
- `crates/`: Rust workspace crates — `server` (API + bins), `db` (SQLx models/migrations), `executors`, `services`, `utils`, `git` (Git operations), `api-types` (shared API types for local + remote), `review` (PR review tool), `deployment`, `local-deployment`, `remote`.
- `packages/local-web/`: Local React + TypeScript app entrypoint (Vite, Tailwind). Shell source in `packages/local-web/src`.
- `packages/remote-web/`: Remote deployment frontend entrypoint.
- `packages/web-core/`: Shared React + TypeScript frontend library used by local + remote web (`packages/web-core/src`).
- `shared/`: Generated TypeScript types (`shared/types.ts`, `shared/remote-types.ts`) and agent tool schemas (`shared/schemas/`). Do not edit generated files directly.
- `assets/`, `dev_assets_seed/`, `dev_assets/`: Packaged and local dev assets.
- `npx-cli/`: Files published to the npm CLI package.
- `scripts/`: Dev helpers (ports, DB preparation).
- `docs/`: Documentation files.

### Crate-specific guides
- [`crates/remote/AGENTS.md`](crates/remote/AGENTS.md) — Remote server architecture, ElectricSQL integration, mutation patterns, environment variables.
- [`docs/AGENTS.md`](docs/AGENTS.md) — Mintlify documentation writing guidelines and component reference.
- [`packages/local-web/AGENTS.md`](packages/local-web/AGENTS.md) — Web app design system styling guidelines.

## Managing Shared Types Between Rust and TypeScript

ts-rs allows you to derive TypeScript types from Rust structs/enums. By annotating your Rust types with #[derive(TS)] and related macros, ts-rs will generate .ts declaration files for those types.
When making changes to the types, you can regenerate them using `pnpm run generate-types`
Do not manually edit shared/types.ts, instead edit crates/server/src/bin/generate_types.rs

For remote/cloud types, regenerate using `pnpm run remote:generate-types`
Do not manually edit shared/remote-types.ts, instead edit crates/remote/src/bin/remote-generate-types.rs (see crates/remote/AGENTS.md for details).

## Build, Test, and Development Commands
- Install: `pnpm i`
- Run dev (web app + backend with ports auto-assigned): `pnpm run dev`
- Backend (watch): `pnpm run backend:dev:watch`
- Web app (dev): `pnpm run local-web:dev`
- Type checks: `pnpm run check` (frontend + all backend Rust workspaces) and `pnpm run backend:check` (all backend Rust workspaces, including `crates/remote`)
- Rust tests: `cargo test --workspace`
- Generate TS types from Rust: `pnpm run generate-types` (or `generate-types:check` in CI)
- Prepare SQLx (offline): `pnpm run prepare-db`
- Prepare SQLx (remote package, postgres): `pnpm run remote:prepare-db`
- Local NPX build: `pnpm run build:npx` then `pnpm pack` in `npx-cli/`
- Format code: `pnpm run format` (runs `cargo fmt` for all backend Rust workspaces + web-core/web Prettier)
- Lint: `pnpm run lint` (runs web/ui ESLint + `cargo clippy` for all backend Rust workspaces)

## Before Completing a Task
- Run `pnpm run format` to format all Rust workspaces and web code.

## Coding Style & Naming Conventions
- Rust: `rustfmt` enforced (`rustfmt.toml`); group imports by crate; snake_case modules, PascalCase types.
- TypeScript/React: ESLint + Prettier (2 spaces, single quotes, 80 cols). PascalCase components, camelCase vars/functions, kebab-case file names where practical.
- Keep functions small, add `Debug`/`Serialize`/`Deserialize` where useful.

## Testing Guidelines
- Rust: prefer unit tests alongside code (`#[cfg(test)]`), run `cargo test --workspace`. Add tests for new logic and edge cases.
- Web app: ensure `pnpm run check` and `pnpm run lint` pass. If adding runtime logic, include lightweight tests (e.g., Vitest) in the same directory.

## Security & Config Tips
- Use `.env` for local overrides; never commit secrets. Key envs: `FRONTEND_PORT`, `BACKEND_PORT`, `HOST` 
- Dev ports and assets are managed by `scripts/setup-dev-environment.js`.

<!-- task-ai-room:start -->
## Shared AI Room — mandatory

Before analysis or the first project tool call, search from the current working directory upward for the nearest `.ai-room/ROOM.md`. A room located in a parent project root still applies inside a nested module. If found, its workflow is required, not optional documentation.

Use the directory containing `.ai-room` as the room root:
1. Read `.ai-room/sessions/INDEX.md` first when present, then `ROOM.md`, `.ai-room/library/owner-working-rules.md`, and the room files they require. Check active (`진행중`) sessions before choosing files to edit.
2. Create your own unique `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse or edit another AI's session.
3. Before project work, write this exact header shape and the first checkpoint: `# Session: title`, `- Agent: name`, `- Module: area`, `- Status: 진행중`, `- Started: YYYY-MM-DD HH:MM (timezone)`.

During work:
- Create the session once at work start. Update it before every user-facing final response and at meaningful transitions, cancellation, or completion; do not rewrite it merely because 5 minutes passed.
- Send the user a visible progress report when work starts and at least every 5 minutes until completion. Session-file writes do not count. State what finished, what is running, blockers, and what comes next; do not repeat generic waiting text. Never use one foreground tool call or wait that can block reporting for 4 minutes or longer; run long work asynchronously and poll at most every 60 seconds. Warn before a truly uninterruptible operation and report immediately afterward.
- Record Goal, checkpoint evidence, decisions and approval state, blockers, failed approaches, changed files, verification, and ordered next steps.
- Do not edit files claimed by another active session without user coordination.
- Never edit `.ai-room/tasks.md` or `.ai-room/decisions.md`; Task AI Platform derives them.
- After code or executable-configuration changes, read `.ai-room/library/adversarial-code-review-protocol.md` and complete its two-independent-critic, evidence-driven review before claiming completion.

Before the final response, set `Status` to `완료`, `중단`, or `보류`, update the handoff, regenerate `sessions/INDEX.md` when its documented command exists, then add the completion marker required by `ROOM.md` as the final line. AI Room records are private runtime data and must never be committed to Git.
<!-- task-ai-room:end -->
