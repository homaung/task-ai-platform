# Session: Active session sync and AI-managed decisions

## Goal

- Synchronize active remote AI session checkpoints to local storage without deleting server records.
- Make `tasks.md` and `decisions.md` local AI-managed views derived from session records.
- Safely upgrade stale remote room instructions, support explicit stopped-session overrides, and keep the desktop app available in the background.

## Status

- In progress.

## Constraints

- Preserve existing user changes.
- Do not commit or push.
- Verify with tests, a release build, and the configured `server_250` room.

## Current findings

- The configured local room instructions and the server room instructions have diverged.
- The server has an active Claude session updated on 2026-07-27, but the local app only imports completed sessions.
- Task AI Platform was not running, so local summarization could not run.

## Checkpoint: implementation connected

- Active remote session files now fast-forward into local storage when the remote content extends the local checkpoint; incomplete server records are not deleted.
- Local `tasks.md` and `decisions.md` are AI-managed views. The decision view accepts only evidence-backed, approved or implemented durable decisions.
- Session stop overrides are stored separately in `session-overrides.json` and exposed through the AI Room API/UI.
- Active server instructions upgrade in place without deleting session records.
- Windows desktop behavior now includes login autostart, background-on-close, a tray menu, and single-instance foreground activation.
- Server AI Room tests and the Rust desktop compile check pass after the first implementation pass.

## Completion

- Active remote checkpoints are mirrored locally every 15 seconds and summarized after two quiet minutes without deleting an incomplete server room.
- `tasks.md` now preserves the newest explicit follow-up even when the local model omits it.
- `decisions.md` now preserves evidence-backed durable decisions, explicitly stated session decisions, and a link to the migrated legacy decision source.
- Users can mark a forcibly terminated session as stopped and undo that override from the room UI.
- Existing remote instructions are upgraded in place; room sessions and library documents remain intact.
- Windows login autostart registration, tray access, background close behavior, and single-instance activation are implemented.
- Verification: Rust formatting passed; all 21 AI Room tests passed; release build succeeded; the rebuilt executable is running; the configured room produced the expected `exp34+` task and explicit discard decisions.
- No commit or push was performed.

<!-- task-ai-room:complete -->
