# 다른 Windows PC에서 개발 이어가기

## 목적

회사 PC에서 진행한 Task AI Platform 개발을 집 PC에서도 같은 프로젝트
맥락으로 이어간다. Codex 자체의 로컬 대화 DB를 복제하는 대신, Git에 포함된
상세 AI Room 기록을 새 세션이 읽어 작업 상태를 복원하도록 한다.

## Git에 보관하는 것

- 추적되는 소스 코드와 문서
- `.ai-room/context.md`
- `.ai-room/decisions.md`
- `.ai-room/tasks.md`
- `.ai-room/sessions/*.md`
- `.ai-room/library/*.md`
- `AGENTS.md`와 `CLAUDE.md`의 관리 블록

## Git에 보관하지 않는 것

- GitHub, Codex, Claude, Ollama 로그인 토큰
- SSH 개인 키와 원격 서버 자격 증명
- Task AI Platform의 컴퓨터별 로컬 데이터베이스
- `target/`, `node_modules/`, 설치 파일과 기타 생성 결과물
- `%USERPROFILE%\.codex` 전체

두 컴퓨터에서 `%USERPROFILE%\.codex`를 Google Drive 등으로 실시간
동기화하면 인증 정보 노출과 로컬 DB 충돌 위험이 있으므로 사용하지 않는다.

## 회사 PC에서 작업을 넘기기 전

1. 현재 AI 세션 파일을 갱신한다.
2. 목표, 완료 사항, 변경 파일, 실행한 검증, 미완료 작업, 차단 요인을 구체적으로
   기록한다.
3. 작업이 완전히 끝났으면 세션 파일 마지막 줄에
   `<!-- task-ai-room:complete -->`를 추가한다. 단순히 다른 PC로 넘기는
   중이라면 완료 표시를 하지 않고 `Status: Handoff`로 둔다.
4. 비밀 정보와 회사 반출 금지 데이터가 포함되지 않았는지 확인한다.
5. 변경을 커밋하고 비공개 GitHub 저장소의 `main`에 푸시한다.

## 집 PC 최초 설정

```powershell
git clone https://github.com/homaung/task-ai-platform.git C:\AI-Workspace\task-ai-platform
cd C:\AI-Workspace\task-ai-platform
winget install --id Ollama.Ollama -e
ollama pull qwen3.5:4b
```

Rust, Node.js 20 이상, pnpm 8 이상도 설치한다. GitHub와 Codex/Claude 로그인,
SSH 키는 집 PC에서 별도로 구성한다.

Task AI Platform을 실행한 후 이 저장소를 로컬 전용 AI Room으로 등록한다.
AI Room 데이터베이스는 컴퓨터별 상태이므로 Git clone만으로 앱 목록에 방이
자동 등록되지는 않는다. 단, 저장소의 `.ai-room` 기록은 그대로 보존된다.

## 매번 작업을 전환할 때

```powershell
git status
git pull --ff-only
```

그다음 저장소 루트에서 Codex 또는 Claude를 시작한다. 새 AI는 먼저
`AGENTS.md`, `.ai-room/ROOM.md`, `context.md`, `decisions.md`, `tasks.md`,
관련 library 문서와 최신 session 파일을 읽어야 한다.

작업 후에는 동일하게 상세 세션 기록을 남기고 커밋·푸시한다. 두 컴퓨터에서
동시에 같은 브랜치를 수정하지 않는다. 동시에 작업해야 한다면 별도 브랜치나
worktree를 사용하고 검증 후 `main`에 합친다.

## 기대되는 연속성

- Codex의 대화창 원문과 UI 상태가 다른 PC에 그대로 나타나는 방식은 아니다.
- 대신 목표, 이유, 결정, 변경 파일, 검증 결과와 다음 행동을 AI Room이
  제공하므로 새 세션이 실질적인 작업 상태를 복원한다.
- 원문 대화까지 휴대해야 하는 기능은 향후 암호화된 세션 내보내기/가져오기로
  별도 설계한다.
