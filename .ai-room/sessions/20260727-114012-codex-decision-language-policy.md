# Session: Decision record language policy

## Goal

- Keep shared `decisions.md` records in Korean.
- Allow Claude, Codex, and other agents to write session checkpoints in whichever language preserves their reasoning and handoff context most accurately.

## Status

- In progress.

## Constraints

- Do not commit or push.
- Preserve the existing automatic task and decision workflow.

## Initial checkpoint

- The decision summarizer already requests natural Korean, but deterministic extraction can copy an English explicit-decision line verbatim.
- Room instructions currently do not state the intended difference between shared document language and agent session language.

## Completion

- Room instruction version 7 now states that session checkpoints may use the active AI's most accurate working language.
- The same instruction identifies `decisions.md` as an owner-and-AI shared Korean document.
- The local decision prompt now requires Korean translation for title, decision, and rationale while preserving technical identifiers.
- The renderer rejects non-Korean explanatory decision entries; deterministic explicit-decision fallback only copies Korean statements.
- Verification: Rust formatting passed, all 21 AI Room tests passed, the release build succeeded, the rebuilt app is running, and the server ROOM instruction contains the new language policy.
- No commit or push was performed.

<!-- task-ai-room:complete -->
