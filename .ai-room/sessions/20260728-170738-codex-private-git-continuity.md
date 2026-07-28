# Session: Private Git continuity and self-hosted AI Room

## Status

Stopped / handoff at the owner's request on 2026-07-28. Do not treat this
session as completed.

## Goal

- Make Task AI Platform development safely continuable from the owner's home
  Windows computer.
- Give this repository its own AI Room instead of reusing a room belonging to a
  managed SSH project.
- Preserve enough technical and product context that a new Claude or Codex
  session can continue without the current chat transcript.
- Publish the complete current worktree to a private GitHub repository after
  verification.

## User intent and constraints

- The owner explicitly requested a private Git repository, a very detailed
  session record, a commit, and a push.
- The reason for the detailed record is that local Codex conversations do not
  automatically provide identical cross-computer continuity.
- The owner was concerned that creating an AI Room for the room-management tool
  itself could create a recursive or circular workflow.
- The safe boundary is: Task AI Platform has one ordinary project room for its
  own source repository; that room records development of the tool, while rooms
  managed by the tool record their respective external projects.
- Do not copy GitHub/Codex/Claude credentials, SSH private keys, local databases,
  build outputs, or `%USERPROFILE%\.codex` into Git.
- The previously identified visual multi-agent products remain review-only.
  Do not adopt their code or architecture until the owner explicitly says
  `확인`.

## Starting repository state

- Checkout: `C:\AI-Workspace\task-ai-platform`
- Branch: `main`
- Starting commit: `39725d2b`
- Original remote: `https://github.com/homaung/task-ai-platform.git`
- GitHub reported the existing repository visibility as `public`.
- The worktree already contained substantial uncommitted implementation from a
  prior session. Those changes belong to the owner and were preserved.
- Initial modified paths:
  - `.ai-room/tasks.md`
  - `Cargo.lock`
  - `crates/db/src/models/ai_room.rs`
  - `crates/server/src/bin/generate_types.rs`
  - `crates/server/src/routes/ai_rooms.rs`
  - `crates/tauri-app/Cargo.toml`
  - `crates/tauri-app/src/main.rs`
  - `packages/web-core/src/pages/ai-rooms/AiRoomsPage.tsx`
  - `packages/web-core/src/shared/lib/api.ts`
  - `pnpm-workspace.yaml`
  - `shared/types.ts`
  - two untracked platform-development session files under
    `.ai-room/sessions/`

## Important discovery: stale room identity

- The repository's `.ai-room/ROOM.md` claimed this source tree belonged to
  `Autolabel_pig_skul` and pointed to
  `server_250:/home/intflow/works/yoloe_skul_pig`.
- The two local session files actually described Task AI Platform development,
  not animal-model work.
- The running Task AI Platform database already had a correct, separate
  `autolabel_animal` room rooted at
  `C:\AI-Workspace\autolabel_animal`, with the same server project and its own
  complete session history.
- Therefore the stale `.ai-room` metadata in this repository was not the
  canonical animal-project room. Reassigning this repository to a dedicated
  platform room does not remove the real animal-project records.

## AI Room creation

- The running local app was reachable on its frontend proxy at `[::1]:2457`.
- A local-only room named `Task AI Platform` was created through the app API.
- New room ID: `7b8d320b-c427-4b37-9dd7-52e98c32050c`
- Local root: `C:\AI-Workspace\task-ai-platform`
- Remote root: not configured
- Current room instruction version: 7
- Initialization updated `room.json`, `ROOM.md`, and the managed room blocks in
  `AGENTS.md` and `CLAUDE.md`, while preserving the two existing platform
  session files.
- `.ai-room/context.md` was replaced with detailed durable product context at
  the owner's explicit request.
- `.ai-room/library/CROSS_COMPUTER_CONTINUITY.md` was added as the reusable
  company-to-home handoff procedure.

## Existing implementation being published

The uncommitted implementation predating this session includes:

- Active SSH session checkpoints fast-forward into local storage instead of
  waiting for the whole chat or session to finish.
- Incomplete remote records are retained; server cleanup happens only after all
  sessions are complete and the merge has no conflicts.
- `tasks.md` and `decisions.md` are AI-managed local dashboards built from
  stable session checkpoints.
- Task and decision summarization uses local Ollama and `qwen3.5:4b`.
- Decision rendering requires Korean explanatory text while preserving
  technical identifiers.
- Explicit stopped-session overrides prevent forcibly terminated work from
  being treated as active or complete.
- Remote `ROOM.md`, `AGENTS.md`, and `CLAUDE.md` instructions can be upgraded
  safely without deleting active records.
- The Windows application starts in the background at login, remains available
  in the system tray, hides instead of exiting on window close, and enforces a
  single running instance.
- The AI Room UI distinguishes owner-editable project context from AI-managed
  task and decision views.

## Prior verification evidence

The two preserved completed session files report:

- Rust formatting passed.
- All 21 AI Room tests passed.
- The desktop release build succeeded.
- The rebuilt executable was running.
- The configured SSH room produced expected task and decision summaries.

These results describe the prior implementation checkpoint. Relevant checks
must be rerun before the new commit because room metadata and handoff
documentation have since changed.

## Cross-computer operating model

- Git is the source of truth for source code and portable `.ai-room` records.
- Each computer clones the private repository and configures its own GitHub,
  Codex/Claude, Ollama, and SSH authentication.
- Task AI Platform's application database is machine-local. On a new computer,
  the cloned repository must be registered as a local-only room in the app;
  its existing `.ai-room` files remain the portable history.
- A new Codex conversation will not be the identical UI conversation. It gains
  equivalent working context by reading the detailed room records.
- The company PC must push a final or handoff checkpoint before the home PC
  pulls. Avoid concurrent edits to `main`.

## GitHub authentication blocker

- `gh` version 2.96.0 is installed.
- `gh auth status` reports that the token for account `homaung` is invalid.
- A browser login was launched with
  `gh auth login -h github.com -p https -w`; authentication still needs to be
  confirmed before changing repository visibility or pushing.

## Stop checkpoint

- The owner said to continue later.
- No commit was created.
- No push was performed.
- The existing GitHub repository remains public.
- The dedicated local-only `Task AI Platform` AI Room was created and its
  detailed context, cross-computer procedure, and this handoff session exist in
  the working tree.
- The first formatting attempts were blocked by pnpm's non-interactive
  `node_modules` purge prompt. An invalid temporary `allowBuilds` placeholder
  block was removed from `pnpm-workspace.yaml`.
- A retry with `CI=true` was manually aborted by the owner. It must not be
  counted as a successful format or dependency-install run. Before resuming,
  inspect `git status` and dependency state because the interrupted pnpm
  operation may have partially rebuilt `node_modules`.
- Do not add the completion marker to this session until repository privacy,
  verification, commit, and push have all succeeded.

## Planned verification

1. Inspect the final diff and ensure no secret or machine-local credential was
   added.
2. Run repository formatting as required by `AGENTS.md`.
3. Run the focused AI Room Rust test suite.
4. Run frontend type checking for the affected web package.
5. Confirm generated TypeScript types match Rust declarations.
6. Confirm the GitHub repository is private.
7. Stage only the intended current worktree, commit on `main`, and push to the
   private `origin`.

## Candidate decisions

- Approved by the owner: use a private Git repository to carry this project's
  source and detailed AI Room records between company and home computers.
- Approved by the owner: create a dedicated AI Room for Task AI Platform and
  write unusually detailed handoff records.
- Implemented boundary: the platform's own room is local-only and separate from
  every external project room it manages.

## Next action

When the owner resumes: inspect `git status`, confirm `gh auth status`, restore
or install dependencies safely if needed, finish verification, make the
repository private, commit the complete intended worktree, and push `main`.
