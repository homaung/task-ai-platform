AGENTS.md

<!-- task-ai-room:start -->
## Shared AI Room — mandatory

Before analysis or the first project tool call, search from the current working directory upward for the nearest `.ai-room/ROOM.md`. A room located in a parent project root still applies inside a nested module. If found, its workflow is required, not optional documentation.

Use the directory containing `.ai-room` as the room root:
1. Read `.ai-room/sessions/INDEX.md` first when present, then `ROOM.md`, `.ai-room/library/owner-working-rules.md`, and the room files they require. Check active (`진행중`) sessions before choosing files to edit.
2. Create your own unique `.ai-room/sessions/YYYYMMDD-HHMMSS-<agent>-<short-id>.md`. Never reuse or edit another AI's session.
3. Before project work, write this exact header shape and the first checkpoint: `# Session: title`, `- Agent: name`, `- Module: area`, `- Status: 진행중`, `- Started: YYYY-MM-DD HH:MM (timezone)`.

During work:
- Create the session once at work start. Update it before every user-facing final response and at meaningful transitions, cancellation, or completion; do not rewrite it merely because 5 minutes passed.
- Send the user a visible progress report when work starts and at least every 5 minutes until completion. Session-file writes do not count. State what finished, what is running, blockers, and what comes next; do not repeat generic waiting text. Never use one foreground tool call or wait that can block reporting for 4 minutes or longer; run long work asynchronously and poll at most every 60 seconds. Warn before a truly uninterruptible operation and report immediately afterward.
- Record Goal, checkpoint evidence, decisions and approval state, blockers, failed approaches, changed files, verification, and ordered next steps.
- Do not edit files claimed by another active session without user coordination.
- Never edit `.ai-room/tasks.md` or `.ai-room/decisions.md`; Task AI Platform derives them.
- After code or executable-configuration changes, read `.ai-room/library/adversarial-code-review-protocol.md` and complete its two-independent-critic, evidence-driven review before claiming completion.

Before the final response, set `Status` to `완료`, `중단`, or `보류`, update the handoff, regenerate `sessions/INDEX.md` when its documented command exists, then add the completion marker required by `ROOM.md` as the final line. AI Room records are private runtime data and must never be committed to Git.
<!-- task-ai-room:end -->
