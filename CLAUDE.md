AGENTS.md

<!-- task-ai-room:start -->
## Shared AI Room — mandatory

Before analysis or the first project tool call, search from the current working directory upward for the nearest `.ai-room/ROOM.md`. A room located in a parent project root still applies inside a nested module. If found, its workflow is required, not optional documentation.

Use the directory containing `.ai-room` as the room root:
1. Read `.ai-room/sessions/INDEX.md` first when present, then `ROOM.md`, `.ai-room/library/owner-working-rules.md`, and the room files they require. Check active (`진행중`) sessions before choosing files to edit.
2. Give this chat its own unique `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<random-id>/` directory. When `TASK_AI_SESSION_DIR` exists, use exactly that absolute platform-created directory. If `/clear`, fork, resume, or another in-process action starts a new conversation, ignore the inherited directory and create a new random-id directory. A new chat window or fork is always a new conversation, even when inherited context names an older session directory. Generate at least 8 random hexadecimal characters for `<random-id>` and, if that directory already exists, generate another. Never continue in an inherited directory and never use another chat's directory.
3. Before project work, create `000001-start.md` there when `TASK_AI_SESSION_DIR` is absent. Except after an in-process `/clear`, fork, or resume, when it exists the platform already created `000001-start.md`; never modify it and create `000002-checkpoint.md` for the first user answer. For an in-process new conversation, treat the inherited variable as stale, create the new random-id directory with its own create-new `000001-start.md`, and use that new path for every later checkpoint with this exact header shape: `# Session: title`, `- Agent: name`, `- Module: area`, `- Status: 진행중`, `- Started: YYYY-MM-DD HH:MM (timezone)`.

During work:
- For every user message, before sending the corresponding AI answer, add the next zero-padded Markdown checkpoint; this applies even to short answers, questions, planning, and no-code turns. Also add checkpoints at meaningful transitions. Never rewrite, rename, or delete an existing checkpoint.
- Send the user a visible progress report when work starts and at least every 5 minutes until completion. Session-file writes do not count. State what finished, what is running, blockers, and what comes next; do not repeat generic waiting text. Never use one foreground tool call or wait that can block reporting for 4 minutes or longer; run long work asynchronously and poll at most every 60 seconds. Warn before a truly uninterruptible operation and report immediately afterward.
- Repeat the discoverable header in every checkpoint and record Goal, checkpoint evidence, decisions and approval state, blockers, failed approaches, changed files, verification, and ordered next steps.
- Do not edit files claimed by another active session without user coordination.
- Never edit `.ai-room/tasks.md` or `.ai-room/decisions.md`; Task AI Platform derives them.
- After code or executable-configuration changes, read `.ai-room/library/adversarial-code-review-protocol.md` and complete its two-independent-critic, evidence-driven review before claiming completion.

Before the final response, create one final checkpoint with `Status: 완료`, `중단`, or `보류`, use the one checkpoint required for this user message as the final checkpoint; do not create a second file for the same answer. Put the completion marker required by `ROOM.md` on that file's final line. Then regenerate `sessions/INDEX.md` when its documented command exists without modifying the checkpoint. AI Room records are private runtime data and must never be committed to Git.
<!-- task-ai-room:end -->
