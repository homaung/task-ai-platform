# AI Room: Autolabel_pig_skul

Room ID: `08e10887-0a9f-45fe-993d-f934d98592c8`
Instruction version: 1

## Required session workflow

1. Before doing any work, read `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, and the newest files in `.ai-room/sessions/`.
2. Create one new session file named `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse another session's filename.
3. Record the goal, assumptions, important commands, files changed, verification results, decisions, blockers, and concrete next steps. Update it during the session, not only at the end.
4. Update `tasks.md` when task status changes. Update `context.md` only for durable project facts. Append architectural decisions to `decisions.md`; do not rewrite history.
5. Never store secrets, tokens, private keys, or raw credentials in room files.
6. Before ending, make the session file sufficient for another Claude or Codex session to continue without relying on chat history.

## Server privacy

When this room is prepared on its SSH server, all `.ai-room` files there are temporary. The Task AI Platform copies completed session files to the local root and removes the server-side room files after a conflict-free sync. Do not assume earlier session files remain on the server.

## Room endpoints

- Local root: `C:\AI-Workspace\task-ai-platform`
- Remote root: `server_250:/home/intflow/works/yoloe_skul_pig`

The Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.
