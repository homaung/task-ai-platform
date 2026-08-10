# Task AI Platform

Task AI Platform is a local-first desktop record manager for handing one
software project between Claude, Codex, and other coding-agent sessions. The
agents continue working in their normal CLI or desktop environment; this app
connects a local project root and an optional SSH project root into one logical
**AI Room**.

Each room installs a managed `.ai-room` protocol on the local project. For SSH
work, the app temporarily prepares the same protocol and managed
`AGENTS.md`/`CLAUDE.md` blocks on the server. After an agent marks its session
complete, the app safely synchronizes its records locally and removes the
temporary server records.

## Features

- One project equals one AI Room.
- Local root plus optional SSH alias and remote root.
- Shared project context, architectural decisions, task status, and session
  handoffs.
- Reusable AI-authored methods, rules, prompts, and checklists under
  `.ai-room/library/*.md`.
- Safe three-way merging for library documents changed locally or through SSH.
- Explicit completion markers prevent cleanup while an agent is working.
- Conflicting records are preserved rather than overwritten.
- A local `qwen3.5:4b` Ollama model creates one validated task-status line per
  completed session.
- Completed server records are retained locally and removed from the server.

## Windows usage

Run the stable desktop artifact at:

```text
C:\AI-Workspace\task-ai-platform\Task AI Platform.exe
```

The Cargo build output is named `task-ai-platform.exe`. The stable artifact
above is copied from that build so it remains easy to find without relying on a
desktop shortcut.

To enable local session summaries on a new Windows computer:

```powershell
winget install --id Ollama.Ollama -e
ollama pull qwen3.5:4b
```

## AI Room workflow

1. Create a room and choose its local project root.
2. Optionally select a host alias from the user's existing SSH configuration and
   provide the matching remote project root.
3. For local work, run Claude or Codex directly in the local project.
4. For SSH work, select **Prepare server work** before starting the agent.
5. Leave Task AI Platform running. It detects completed sessions, synchronizes
   records locally, and cleans the temporary server room.

Ask an agent to “save this method/rule/checklist in the AI Room” when later
sessions should reuse it. The agent writes a focused Markdown document to
`.ai-room/library/`; the desktop app lists it under **Room documents** for
reading and editing.

Room files must never contain secrets, tokens, private keys, credentials, or
personal data.

## Continue on another Windows computer

Git synchronizes source code only. `.ai-room/` contains private project context,
decisions, task state, and session records, so it is excluded from Git and must
not be committed even when the repository itself is private.

On the other computer:

```powershell
git clone https://github.com/homaung/task-ai-platform.git C:\AI-Workspace\task-ai-platform
cd C:\AI-Workspace\task-ai-platform
```

Configure GitHub, Codex or Claude, Ollama, and SSH locally, then register the
cloned folder as a local-only AI Room. Room continuity between computers
requires a separate encrypted export or synchronization mechanism; copying
Codex state or committing `.ai-room/` is not supported.

## Development

Prerequisites:

- Rust
- Node.js 20 or newer
- pnpm 8 or newer

Common checks:

```powershell
cargo test -p server ai_rooms --lib
pnpm --filter @vibe/web-core run check
pnpm --filter @vibe/local-web run build
```

Build the desktop executable:

```powershell
cargo build -p task-ai-platform --release
```

The raw Cargo output is `target\release\task-ai-platform.exe`.

## Upstream

Task AI Platform is derived from
[Vibe Kanban](https://github.com/BloopAI/vibe-kanban) v0.1.36 under the Apache
License 2.0. See [UPSTREAM.md](UPSTREAM.md) for the exact relationship,
retained compatibility components, and attribution details. The original
license is preserved in [LICENSE](LICENSE).
