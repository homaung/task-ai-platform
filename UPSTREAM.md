# Upstream and attribution

Task AI Platform is an independent derivative of
[Vibe Kanban](https://github.com/BloopAI/vibe-kanban), based on the upstream
v0.1.36 source.

The upstream project is licensed under the Apache License 2.0. Its original
copyright and license terms are preserved in [LICENSE](LICENSE) and in inherited
source files where applicable. Task AI Platform is not an official Vibe Kanban
release and is not affiliated with or endorsed by the upstream maintainers.

## What this derivative changes

Task AI Platform changes the product focus from a Kanban-oriented coding-agent
runner to a local-first project record and handoff manager:

- one project is represented by one AI Room;
- local and optional SSH roots share a managed `.ai-room` protocol;
- completed server records are synchronized locally and removed from the server;
- Claude, Codex, and other agents share context, decisions, task summaries,
  reusable room documents, and session handoffs;
- a local Ollama model summarizes completed sessions without sending their
  contents to a cloud model;
- the desktop product name, application identifier, icons, Rust package, and
  executable use Task AI Platform branding.

## Inherited compatibility code

Some source modules, documentation, protocol constants, data-directory names,
and NPX/MCP compatibility components still contain the upstream `Vibe Kanban`
name. They are retained where renaming would break existing data, integrations,
or attribution. They do not identify the Task AI Platform desktop executable.

The desktop Rust package and binary are named `task-ai-platform`; the stable
local launch artifact is `Task AI Platform.exe`.
