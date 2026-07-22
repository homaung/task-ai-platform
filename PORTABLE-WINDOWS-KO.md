# Windows 어디서나 이어서 개발하기

> 이 문서는 레거시 백업/이동 방식입니다. 현재 권장 방식은 Google Drive를
> 실제 작업 루트로 쓰는 대신, 로컬 데스크톱 앱에서 로컬 루트와 SSH 서버
> 루트를 하나의 AI Room으로 연결하고 `.ai-room` 기록을 동기화하는 것입니다.

이 프로젝트의 Google Drive 복제본은 소스와 Git 이력만 동기화합니다.
`node_modules`, Rust `target`, 컴파일러와 SDK는 PC 로컬에 둡니다. 이 방식은
Google Drive의 대량 파일 동기화와 Git 잠금 파일 충돌을 피합니다.

## 새 Windows PC에서 시작

1. Google Drive for desktop을 설치하고 동기화가 끝날 때까지 기다립니다.
2. Drive의 `Workspace\task-ai-platform-portable` 폴더에서
   `Open Workspace.cmd`를 실행합니다.
3. 소스가 `%USERPROFILE%\AI-Workspace\task-ai-platform`에 복제됩니다.
4. 처음 한 번 로컬 복제본의 `Setup Development.cmd`를 실행합니다.
5. 이후에는 `Open Development Shell.cmd`를 실행해 개발합니다.

`Setup Development.cmd`는 다음 항목을 준비합니다.

- Git
- Node.js 24.18.0
- pnpm 10.13.1
- Rust nightly-2025-12-04
- Visual Studio 2022 C++ Build Tools
- LLVM/libclang
- JavaScript 및 Rust 의존성

도구와 Rust 빌드 캐시는 `%LOCALAPPDATA%\TaskAIPlatform`에 저장됩니다.

## 다른 PC로 작업 넘기기

작업을 마칠 때 Drive 폴더의 `Save Workspace.cmd`를 실행합니다. 변경 파일을
확인한 뒤 커밋 메시지를 입력하면 Drive의 중앙 Git 저장소로 푸시됩니다.
Google Drive 동기화가 완료된 것을 확인한 다음 다른 PC에서
`Open Workspace.cmd`를 실행하세요.

동시에 두 PC에서 수정하지 마세요. Drive 동기화가 끝나기 전에 다른 PC에서
열면 Git 이력이 충돌할 수 있습니다.

## 앱만 실행

개발 도구가 필요하지 않은 PC에서는 Drive 폴더의
`Run Task AI Platform.cmd`를 실행합니다. 동봉된 Windows x64 네이티브 앱이
바로 실행됩니다.

## 보안 정보

SSH 개인키, `~/.ssh/config`, Claude/Codex 로그인 토큰, `.env` 파일은 Drive에
복사하지 않습니다. 새 PC에서 원격 서버를 사용할 때는 그 PC의
`%USERPROFILE%\.ssh`에 키와 Host 설정을 별도로 준비해야 합니다.
