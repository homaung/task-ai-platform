# Context

## Product identity

- Product name: Task AI Platform.
- This repository contains the desktop application itself. It must have its own
  AI Room and must never reuse the AI Room of a project that the application
  manages.
- The application is derived from Vibe Kanban v0.1.36 under Apache License 2.0.
  Upstream attribution and retained compatibility details live in
  `UPSTREAM.md`.
- The owner prefers release labels with one dot, such as `v0.11`, `v0.12`, and
  `v0.2`, rather than three-part versions such as `v0.2.0`.

## Product purpose

- Provide a local-first desktop record manager that lets Claude, Codex, and
  other coding agents continue work on one software project across sessions and
  computers.
- One managed project corresponds to one logical AI Room.
- A room may connect one local project root to one optional SSH alias and remote
  project root.
- Agents continue working in their normal CLI, desktop app, or SSH environment.
  Task AI Platform manages durable context and handoff records rather than
  replacing the agents' own interfaces.

## Record ownership

- `context.md` contains durable project context written or explicitly approved
  by the owner.
- Session files contain detailed checkpoints written by the active AI during
  work. A long-lived chat must still write checkpoints after meaningful work
  units and at least every ten minutes of active work.
- `tasks.md` and `decisions.md` are generated locally from stable session
  records. Agents and users should not have to maintain them manually.
- Shared decision explanations are rendered in Korean. Session checkpoints may
  use the language that preserves technical meaning most accurately.
- Reusable rules, prompts, methods, and checklists belong in
  `.ai-room/library/*.md`, one focused topic per file.

## Privacy and synchronization

- Room documents must never contain secrets, tokens, private keys, raw
  credentials, personal data, or generated binaries.
- Active SSH session checkpoints are copied to local storage without deleting
  the server copy. Server-side temporary room data is removed only when every
  remote session is complete and the merge is conflict-free.
- Task and decision summarization uses the local Ollama service with
  `qwen3.5:4b`; session contents are not sent to a cloud summarization model.
- For cross-computer development, source code and `.ai-room` records are
  synchronized through the private Git repository. Machine-local application
  databases, authentication state, SSH private keys, build outputs, and Codex
  internal state are not synchronized through Git.

## Current development environment

- Primary company checkout: `C:\AI-Workspace\task-ai-platform`.
- Stable Windows artifact: `C:\AI-Workspace\task-ai-platform\Task AI Platform.exe`.
- Cargo release artifact:
  `C:\AI-Workspace\task-ai-platform\target\release\task-ai-platform.exe`.
- The application normally remains running in the Windows tray so automatic
  room synchronization and local summarization continue after its window is
  closed.
- The repository currently uses `main` as its working and default branch.

## Current product direction

- The core value is reliable project memory: active checkpoints, completed
  sessions, AI-maintained task and decision dashboards, reusable room
  documents, conflict preservation, and privacy-safe SSH cleanup.
- The next large concept under consideration is a visual multi-agent room in
  which Claude, Codex, and subagents appear as characters, communicate, and
  divide work while producing one shared session record.
- Existing products such as Pixel Agents, AI Town, PixelDesk, and Agent Office
  were identified as possible references. No design or code should be adopted
  from them until the owner reviews the examples and explicitly says
  `확인`.
