# AI Room: Task AI Platform

Room ID: `7b8d320b-c427-4b37-9dd7-52e98c32050c`
Instruction version: 7

## Required session workflow

1. Before doing any work, read `.ai-room/context.md`, `.ai-room/decisions.md`, `.ai-room/tasks.md`, every relevant file in `.ai-room/library/`, every additional Markdown instruction directly under `.ai-room/`, and the newest files in `.ai-room/sessions/`.
2. Create one new session file named `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse another session's filename.
3. Record the goal, assumptions, important commands, files changed, verification results, candidate decisions, blockers, and concrete next steps. Write the first checkpoint immediately. Update it after every meaningful work unit and never allow more than 10 minutes of active work without a checkpoint. A chat may remain open for months; checkpoints are work-state records, not chat endings. If the user pauses, stops, or cancels work while you can still write, immediately record the status as Stopped or Cancelled.
4. When the user asks you to remember a reusable method, rule, convention, checklist, prompt, or operating procedure, create or update one focused Markdown file in `.ai-room/library/`. Use a descriptive filename ending in `.md`, keep one topic per file, and make it understandable without chat history. Do not use the library for transient session notes.
5. Do not edit `tasks.md` or `decisions.md`. Task AI Platform reads stable session checkpoints together and locally rebuilds both documents. State results, approval status, next actions, blockers, and whether a candidate decision was actually approved explicitly in the session file.
6. Treat `context.md` as owner-authored project context. Read it but do not edit it unless the user explicitly asks you to change that document.
7. Never store secrets, tokens, private keys, raw credentials, personal data, or generated binaries in room files.
8. Before ending the entire work record, make the session file sufficient for another Claude or Codex session to continue without relying on chat history. After all other writes are finished, add `<!-- task-ai-room:complete -->` as the final line. The completion marker is only for safe server cleanup; local task and decision updates use stable intermediate checkpoints after two quiet minutes.

## Server privacy

While work is active, Task AI Platform copies changing server session checkpoints to local storage without deleting the server files. Once every remote session is complete and the merge is conflict-free, it removes the temporary server room. Task and decision summarization uses only the local Ollama service; session contents are not sent to a cloud model.

## Room endpoints

- Local root: `C:\AI-Workspace\task-ai-platform`
- Remote root: `not configured`

The Task AI Platform manages and synchronizes these records. Claude and Codex perform the project work directly in the selected root.

## Record language

- Session checkpoint files may use whichever language lets the active AI preserve technical meaning and handoff context most accurately; they do not need to be Korean.
- `decisions.md` is shared by the owner and every AI. Task AI Platform renders its explanatory text in Korean and translates session content when necessary. Keep code identifiers, file paths, and product names unchanged when translation would damage their meaning.
