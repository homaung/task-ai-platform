# AI Room: Autolabel_pig_skul

Room ID: `08e10887-0a9f-45fe-993d-f934d98592c8`
Instruction version: 2

## Required session workflow

1. Before doing any work, read `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, and the newest files in `.ai-room/sessions/`.
2. Create one new session file named `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse another session's filename.
3. Record the goal, assumptions, important commands, files changed, verification results, decisions, blockers, and concrete next steps. Update it during the session, not only at the end.
4. Treat `tasks.md` as a concise status dashboard. Never edit, delete, or reorder an existing line. Before ending every session, append exactly one line under `## AI session updates` using `- [x] YYYY-MM-DD HH:MM | <agent> | Done: <summary> | Next: <next action> | Blocked: <none or reason> | Session: sessions/<filename>`. Use `[ ]` instead of `[x]` when the session goal is not complete. Keep the entire entry on one line.
5. Update `context.md` only for durable project facts. Append architectural decisions to `decisions.md`; do not rewrite history.
6. Never store secrets, tokens, private keys, or raw credentials in room files.
7. Before ending, make the session file sufficient for another Claude or Codex session to continue without relying on chat history. After all other writes are finished, add `<!-- task-ai-room:complete -->` as the final line of the session file. The app uses this exact marker to know the session is safe to synchronize and remove from the server.

## Server privacy

When this room is prepared on its SSH server, all `.ai-room` files there are temporary. The Task AI Platform automatically copies completed session files and append-only task updates to the local root, then removes the server-side room files after a conflict-free sync. Do not assume earlier session files remain on the server.

## Room endpoints

- Local root: `C:\AI-Workspace\task-ai-platform`
- Remote root: `server_250:/home/intflow/works/yoloe_skul_pig`

The Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.
