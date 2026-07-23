import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import {
  ArrowsClockwiseIcon,
  CheckCircleIcon,
  CloudIcon,
  FileTextIcon,
  FolderIcon,
  HardDrivesIcon,
  PlusIcon,
  TrashIcon,
  WarningCircleIcon,
} from '@phosphor-icons/react';
import type {
  AiRoom,
  AiRoomSnapshot,
  SshConnectionInfo,
  SshHostsResponse,
} from 'shared/types';

import { aiRoomsApi, sshHostsApi } from '@/shared/lib/api';
import { cn } from '@/shared/lib/utils';

type DocumentKind = 'context' | 'decisions' | 'tasks';

const DOCUMENTS: Array<{ kind: DocumentKind; label: string }> = [
  { kind: 'context', label: '프로젝트 맥락' },
  { kind: 'decisions', label: '결정 기록' },
  { kind: 'tasks', label: '작업 목록' },
];

const MANAGED_ROOM_DOCUMENTS: Partial<Record<string, DocumentKind>> = {
  'context.md': 'context',
  'decisions.md': 'decisions',
  'tasks.md': 'tasks',
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function documentContent(snapshot: AiRoomSnapshot, kind: DocumentKind) {
  return snapshot[kind];
}

function EndpointStatus({
  label,
  configured,
  available,
  installed,
  detail,
}: {
  label: string;
  configured: boolean;
  available: boolean;
  installed: boolean;
  detail: string;
}) {
  const okay = configured && available && installed;
  return (
    <div className="rounded-lg border border-border bg-primary p-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          {okay ? (
            <CheckCircleIcon className="h-5 w-5 shrink-0 text-green-500" />
          ) : (
            <WarningCircleIcon className="h-5 w-5 shrink-0 text-yellow-500" />
          )}
          <span className="font-medium text-high">{label}</span>
        </div>
        <span className="text-xs text-low">
          {!configured
            ? '미설정'
            : !installed
              ? '작업 전 준비 필요'
              : !available
                ? '연결 안 됨'
                : '임시 준비됨'}
        </span>
      </div>
      <p className="mt-2 truncate text-xs text-low" title={detail}>
        {detail}
      </p>
    </div>
  );
}

export function AiRoomsPage() {
  const [rooms, setRooms] = useState<AiRoom[]>([]);
  const [hosts, setHosts] = useState<SshHostsResponse | null>(null);
  const [connection, setConnection] = useState<SshConnectionInfo | null>(null);
  const [selectedRoomId, setSelectedRoomId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<AiRoomSnapshot | null>(null);
  const [selectedDocument, setSelectedDocument] =
    useState<DocumentKind>('context');
  const [draft, setDraft] = useState('');
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [selectedLibraryFile, setSelectedLibraryFile] = useState<string | null>(
    null
  );
  const [libraryDraft, setLibraryDraft] = useState('');
  const [sidePanel, setSidePanel] = useState<'library' | 'sessions'>('library');
  const [showCreate, setShowCreate] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [form, setForm] = useState({
    name: '',
    description: '',
    localRoot: 'C:\\AI-Workspace\\task-ai-platform',
    sshAlias: '',
    remoteRoot: '',
  });

  const loadRooms = useCallback(async () => {
    const nextRooms = await aiRoomsApi.list();
    setRooms(nextRooms);
    setSelectedRoomId((current) =>
      current && nextRooms.some((room) => room.id === current)
        ? current
        : (nextRooms[0]?.id ?? null)
    );
  }, []);

  useEffect(() => {
    void Promise.all([loadRooms(), sshHostsApi.list().then(setHosts)])
      .catch((reason) => setError(errorMessage(reason)))
      .finally(() => setLoading(false));
  }, [loadRooms]);

  const refreshSnapshot = useCallback(async (roomId: string) => {
    setError(null);
    const next = await aiRoomsApi.snapshot(roomId);
    setSnapshot(next);
    return next;
  }, []);

  useEffect(() => {
    if (!selectedRoomId) {
      setSnapshot(null);
      return;
    }
    setLoading(true);
    void refreshSnapshot(selectedRoomId)
      .catch((reason) => setError(errorMessage(reason)))
      .finally(() => setLoading(false));
  }, [refreshSnapshot, selectedRoomId]);

  useEffect(() => {
    if (snapshot) setDraft(documentContent(snapshot, selectedDocument));
  }, [selectedDocument, snapshot]);

  const inspectHost = async (alias: string) => {
    setForm((current) => ({ ...current, sshAlias: alias, remoteRoot: '' }));
    setConnection(null);
    if (!alias) return;
    setBusy(true);
    setError(null);
    try {
      const next = await sshHostsApi.inspect(alias);
      setConnection(next);
      setForm((current) => ({
        ...current,
        remoteRoot: next.repositories[0] ?? '',
      }));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const createRoom = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const room = await aiRoomsApi.create({
        name: form.name,
        description: form.description.trim() || null,
        local_root: form.localRoot,
        ssh_alias: form.sshAlias || null,
        remote_root: form.sshAlias ? form.remoteRoot : null,
      });
      const initialized = await aiRoomsApi.initialize(room.id);
      await loadRooms();
      setSelectedRoomId(room.id);
      setSnapshot(initialized);
      setShowCreate(false);
      setForm((current) => ({
        ...current,
        name: '',
        description: '',
      }));
      setNotice(
        room.ssh_alias
          ? '룸 설명서를 로컬에 설치했습니다. 서버 작업 전에는 서버 작업 준비를 누르세요.'
          : '룸 설명서를 로컬 프로젝트에 설치했습니다.'
      );
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const initializeRoom = async () => {
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    try {
      setSnapshot(await aiRoomsApi.initialize(selectedRoomId));
      setNotice('AGENTS.md, CLAUDE.md와 .ai-room 설명서를 설치했습니다.');
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const syncRoom = async () => {
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await aiRoomsApi.sync(selectedRoomId);
      setSnapshot(result.snapshot);
      setNotice(
        result.conflicts.length
          ? `${result.copied_to_local.length}개 기록을 로컬로 복사했고 ${result.conflicts.length}개 충돌은 서버에 보존했습니다.`
          : `${result.copied_to_local.length}개 새 세션 기록을 로컬로 가져오고 서버의 임시 기록을 삭제했습니다.`
      );
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const importRemoteDocuments = async () => {
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await aiRoomsApi.importRemoteDocuments(selectedRoomId);
      setSnapshot(result.snapshot);
      setSidePanel('library');
      setNotice(
        result.conflicts.length
          ? `${result.copied_to_local.length}개 문서를 가져왔고 ${result.conflicts.length}개 이름 충돌은 보존했습니다.`
          : `${result.copied_to_local.length}개 서버 문서를 룸 문서로 가져왔습니다. 서버 원본은 유지됩니다.`
      );
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const prepareRemote = async () => {
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    try {
      setSnapshot(await aiRoomsApi.prepareRemote(selectedRoomId));
      setNotice(
        '서버에 이번 작업용 임시 설명서와 공용 맥락을 준비했습니다. 작업 기록이 생기면 앱이 자동으로 동기화하고 서버를 정리합니다.'
      );
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const saveDocument = async () => {
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    try {
      const next = await aiRoomsApi.updateDocument(
        selectedRoomId,
        selectedDocument,
        draft
      );
      setSnapshot(next);
      setNotice('공용 문서를 로컬 프로젝트에 저장했습니다.');
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const createLibraryFile = async () => {
    if (!selectedRoomId || !snapshot) return;
    const requested = window.prompt(
      '새 룸 문서 이름을 입력하세요. 예: review-checklist.md'
    );
    if (!requested?.trim()) return;
    const filename = requested.trim().endsWith('.md')
      ? requested.trim()
      : `${requested.trim()}.md`;
    const existing = snapshot.library.find(
      (file) => file.filename === `library/${filename}`
    );
    if (existing) {
      setSelectedLibraryFile(existing.filename);
      setSidePanel('library');
      return;
    }

    setBusy(true);
    setError(null);
    try {
      const title = filename
        .replace(/\.md$/i, '')
        .replace(/[-_]+/g, ' ')
        .trim();
      const next = await aiRoomsApi.updateLibraryFile(
        selectedRoomId,
        filename,
        `# ${title}\n\n## 목적\n\n## 사용 시점\n\n## 절차\n\n`
      );
      setSnapshot(next);
      setSelectedLibraryFile(`library/${filename}`);
      setSidePanel('library');
      setNotice('새 룸 문서를 만들었습니다.');
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const saveLibraryFile = async () => {
    if (!selectedRoomId || !activeLibrary) return;
    setBusy(true);
    setError(null);
    try {
      const managedKind = MANAGED_ROOM_DOCUMENTS[activeLibrary.filename];
      const next = managedKind
        ? await aiRoomsApi.updateDocument(
            selectedRoomId,
            managedKind,
            libraryDraft
          )
        : await aiRoomsApi.updateLibraryFile(
            selectedRoomId,
            activeLibrary.filename.replace('library/', ''),
            libraryDraft
          );
      setSnapshot(next);
      setNotice('룸 문서를 로컬 프로젝트에 저장했습니다.');
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const deleteLibraryFile = async () => {
    if (
      !selectedRoomId ||
      !activeLibrary ||
      !activeLibrary.filename.startsWith('library/')
    )
      return;
    const filename = activeLibrary.filename.replace('library/', '');
    if (!window.confirm(`'${filename}' 룸 문서를 삭제할까요?`)) return;

    setBusy(true);
    setError(null);
    try {
      const next = await aiRoomsApi.deleteLibraryFile(selectedRoomId, filename);
      setSnapshot(next);
      setSelectedLibraryFile(next.library[0]?.filename ?? null);
      setNotice(`'${filename}' 룸 문서를 삭제했습니다.`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const deleteRoom = async () => {
    if (!selectedRoomId || !snapshot) return;
    if (
      !window.confirm(
        `'${snapshot.room.name}' 룸 등록을 삭제할까요? 기록 파일은 남습니다.`
      )
    )
      return;
    setBusy(true);
    try {
      await aiRoomsApi.delete(selectedRoomId);
      setSnapshot(null);
      setSelectedRoomId(null);
      await loadRooms();
      setNotice('룸 등록만 삭제했습니다. 프로젝트 기록 파일은 그대로입니다.');
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const activeSession = useMemo(
    () =>
      snapshot?.sessions.find(
        (session) => session.filename === selectedSession
      ) ?? snapshot?.sessions[0],
    [selectedSession, snapshot]
  );

  const activeLibrary = useMemo(
    () =>
      snapshot?.library.find((file) => file.filename === selectedLibraryFile) ??
      snapshot?.library[0],
    [selectedLibraryFile, snapshot]
  );

  useEffect(() => {
    setLibraryDraft(activeLibrary?.content ?? '');
  }, [activeLibrary]);

  return (
    <div className="flex h-full min-h-0 bg-primary text-normal">
      <aside className="flex w-72 shrink-0 flex-col border-r border-border bg-secondary">
        <div className="border-b border-border p-4">
          <div className="flex items-center gap-2">
            <HardDrivesIcon className="h-6 w-6 text-brand" weight="duotone" />
            <div>
              <h1 className="font-semibold text-high">AI 프로젝트 룸</h1>
              <p className="text-xs text-low">세션 기록 관리자</p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setShowCreate((value) => !value)}
            className="mt-4 flex w-full items-center justify-center gap-2 rounded-md bg-brand px-3 py-2 text-sm font-medium text-white hover:opacity-90"
          >
            <PlusIcon className="h-4 w-4" /> 새 룸
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {rooms.map((room) => (
            <button
              key={room.id}
              type="button"
              onClick={() => setSelectedRoomId(room.id)}
              className={cn(
                'mb-1 w-full rounded-md px-3 py-3 text-left transition-colors',
                selectedRoomId === room.id
                  ? 'bg-primary text-high'
                  : 'text-normal hover:bg-primary/60'
              )}
            >
              <div className="flex items-center gap-2">
                <FolderIcon className="h-4 w-4 shrink-0" />
                <span className="truncate text-sm font-medium">
                  {room.name}
                </span>
              </div>
              <p className="mt-1 truncate pl-6 text-xs text-low">
                {room.ssh_alias ? `${room.ssh_alias} 연결` : '로컬 전용'}
              </p>
            </button>
          ))}
          {!loading && rooms.length === 0 && (
            <p className="p-4 text-center text-sm text-low">
              프로젝트를 룸으로 등록하면 모든 AI 세션이 같은 기록을 읽습니다.
            </p>
          )}
        </div>
      </aside>

      <main className="min-w-0 flex-1 overflow-y-auto">
        {showCreate ? (
          <div className="mx-auto max-w-2xl p-8">
            <h2 className="text-xl font-semibold text-high">
              프로젝트 룸 만들기
            </h2>
            <p className="mt-2 text-sm text-low">
              같은 프로젝트의 로컬 폴더와 SSH 서버 폴더를 하나의 룸으로
              연결합니다.
            </p>
            {error && (
              <div className="mt-5 rounded-md border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-400">
                {error}
              </div>
            )}
            <form
              onSubmit={createRoom}
              className="mt-6 space-y-5 rounded-lg border border-border bg-secondary p-6"
            >
              <label className="block text-sm text-high">
                룸 이름
                <input
                  required
                  value={form.name}
                  onChange={(event) =>
                    setForm({ ...form, name: event.target.value })
                  }
                  className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 outline-none focus:border-brand"
                  placeholder="예: 농장 보고서"
                />
              </label>
              <label className="block text-sm text-high">
                설명 <span className="text-low">(선택)</span>
                <input
                  value={form.description}
                  onChange={(event) =>
                    setForm({ ...form, description: event.target.value })
                  }
                  className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 outline-none focus:border-brand"
                  placeholder="이 룸에서 다루는 프로젝트"
                />
              </label>
              <label className="block text-sm text-high">
                로컬 프로젝트 루트
                <input
                  required
                  value={form.localRoot}
                  onChange={(event) =>
                    setForm({ ...form, localRoot: event.target.value })
                  }
                  className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 font-mono text-sm outline-none focus:border-brand"
                />
              </label>
              <div className="grid gap-4 sm:grid-cols-2">
                <label className="block text-sm text-high">
                  SSH 키 별칭 <span className="text-low">(선택)</span>
                  <select
                    value={form.sshAlias}
                    onChange={(event) => void inspectHost(event.target.value)}
                    className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 outline-none focus:border-brand"
                  >
                    <option value="">로컬만 사용</option>
                    {hosts?.hosts.map((host) => (
                      <option key={host.alias} value={host.alias}>
                        {host.alias} ({host.hostname})
                      </option>
                    ))}
                  </select>
                </label>
                <label className="block text-sm text-high">
                  서버 프로젝트 루트
                  <input
                    list="remote-repositories"
                    required={Boolean(form.sshAlias)}
                    disabled={!form.sshAlias}
                    value={form.remoteRoot}
                    onChange={(event) =>
                      setForm({ ...form, remoteRoot: event.target.value })
                    }
                    className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 font-mono text-sm outline-none disabled:opacity-50 focus:border-brand"
                    placeholder="/home/user/project"
                  />
                  <datalist id="remote-repositories">
                    {connection?.repositories.map((repository) => (
                      <option key={repository} value={repository} />
                    ))}
                  </datalist>
                </label>
              </div>
              <div className="rounded-md bg-primary p-3 text-xs leading-5 text-low">
                생성하면 로컬 프로젝트에 <code>.ai-room</code> 설명서, 룸 문서
                보관함, 세션 폴더를 만듭니다. 서버는 작업을 시작할 때만 임시로
                준비하며, 동기화 뒤 서버 기록을 삭제합니다.
              </div>
              <div className="flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setShowCreate(false)}
                  className="rounded-md border border-border px-4 py-2 text-sm hover:bg-primary"
                >
                  취소
                </button>
                <button
                  disabled={busy}
                  className="rounded-md bg-brand px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
                >
                  {busy ? '연결 중…' : '룸 만들고 설명서 설치'}
                </button>
              </div>
            </form>
          </div>
        ) : snapshot ? (
          <div className="mx-auto max-w-6xl p-6 lg:p-8">
            <header className="flex flex-wrap items-start justify-between gap-4">
              <div>
                <h2 className="text-2xl font-semibold text-high">
                  {snapshot.room.name}
                </h2>
                <p className="mt-1 text-sm text-low">
                  {snapshot.room.description ||
                    'Claude와 Codex가 공유하는 프로젝트 기록 룸'}
                </p>
              </div>
              <div className="flex gap-2">
                <button
                  disabled={busy}
                  onClick={() => void initializeRoom()}
                  className="rounded-md border border-border px-3 py-2 text-sm hover:bg-secondary disabled:opacity-50"
                >
                  설명서 재설치
                </button>
                {snapshot.remote.configured && (
                  <button
                    disabled={
                      busy ||
                      (snapshot.remote.instruction_installed &&
                        snapshot.remote.available)
                    }
                    onClick={() => void prepareRemote()}
                    className="rounded-md border border-border px-3 py-2 text-sm hover:bg-secondary disabled:opacity-50"
                  >
                    서버 작업 준비
                  </button>
                )}
                <button
                  disabled={
                    busy ||
                    (snapshot.remote.configured &&
                      !snapshot.remote.instruction_installed)
                  }
                  onClick={() => void syncRoom()}
                  className="flex items-center gap-2 rounded-md bg-brand px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
                >
                  <ArrowsClockwiseIcon
                    className={cn('h-4 w-4', busy && 'animate-spin')}
                  />{' '}
                  {snapshot.remote.configured ? '지금 동기화' : '새로고침'}
                </button>
              </div>
            </header>

            {(error || notice) && (
              <div
                className={cn(
                  'mt-5 rounded-md border p-3 text-sm',
                  error
                    ? 'border-red-500/40 bg-red-500/10 text-red-400'
                    : 'border-green-500/30 bg-green-500/10 text-green-400'
                )}
              >
                {error || notice}
              </div>
            )}

            <section className="mt-6 grid gap-3 md:grid-cols-2">
              <EndpointStatus
                label="로컬"
                configured={snapshot.local.configured}
                available={snapshot.local.available}
                installed={snapshot.local.instruction_installed}
                detail={snapshot.local.error || snapshot.room.local_root}
              />
              <EndpointStatus
                label="SSH 서버"
                configured={snapshot.remote.configured}
                available={snapshot.remote.available}
                installed={snapshot.remote.instruction_installed}
                detail={
                  snapshot.remote.error ||
                  (snapshot.room.ssh_alias && snapshot.room.remote_root
                    ? `${snapshot.room.ssh_alias}:${snapshot.room.remote_root}`
                    : '연결하지 않음')
                }
              />
            </section>

            <section className="mt-6 rounded-lg border border-border bg-secondary p-5">
              <div className="flex items-start gap-3">
                <CloudIcon className="mt-0.5 h-5 w-5 shrink-0 text-brand" />
                <div>
                  <h3 className="font-medium text-high">사용 방법</h3>
                  <p className="mt-1 text-sm leading-6 text-low">
                    이 앱에서 채팅하지 않습니다. 로컬에서는 Claude 또는 Codex를
                    바로 실행하세요. 서버에서 작업할 때는 먼저{' '}
                    <strong>서버 작업 준비</strong>을 누른 뒤 서버 프로젝트
                    루트에서 실행합니다. 작업이 끝나 세션 기록이 생기면 앱이
                    자동으로 로컬에 보관합니다. AI에게 작업 방식, 규칙, 절차를
                    룸에 저장하라고 하면 <code>.ai-room/library</code>에 문서로
                    남고 오른쪽의 <strong>룸 문서</strong> 목록에 나타납니다.
                    동기화가 끝나면 서버의 임시 <code>.ai-room</code> 파일과
                    관리 안내를 삭제합니다. <strong>지금 동기화</strong>는
                    복구가 필요할 때만 사용하세요.
                  </p>
                </div>
              </div>
            </section>

            <div className="mt-6 grid min-h-[520px] gap-5 lg:grid-cols-[minmax(0,1.5fr)_minmax(280px,0.8fr)]">
              <section className="flex min-h-0 flex-col rounded-lg border border-border bg-secondary">
                <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border p-3">
                  <div className="flex gap-1">
                    {DOCUMENTS.map((document) => (
                      <button
                        key={document.kind}
                        onClick={() => setSelectedDocument(document.kind)}
                        className={cn(
                          'rounded-md px-3 py-2 text-sm',
                          selectedDocument === document.kind
                            ? 'bg-primary text-high'
                            : 'text-low hover:text-high'
                        )}
                      >
                        {document.label}
                      </button>
                    ))}
                  </div>
                  <button
                    disabled={
                      busy ||
                      draft === documentContent(snapshot, selectedDocument)
                    }
                    onClick={() => void saveDocument()}
                    className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-primary disabled:opacity-40"
                  >
                    저장
                  </button>
                </div>
                <textarea
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  spellCheck={false}
                  className="min-h-[440px] flex-1 resize-none bg-transparent p-4 font-mono text-sm leading-6 text-normal outline-none"
                />
              </section>

              <section className="flex min-h-0 flex-col rounded-lg border border-border bg-secondary">
                <div className="flex items-center justify-between gap-2 border-b border-border p-3">
                  <div className="flex gap-1">
                    <button
                      type="button"
                      onClick={() => setSidePanel('library')}
                      className={cn(
                        'rounded-md px-3 py-2 text-sm',
                        sidePanel === 'library'
                          ? 'bg-primary text-high'
                          : 'text-low hover:text-high'
                      )}
                    >
                      룸 문서 {snapshot.library.length}
                    </button>
                    <button
                      type="button"
                      onClick={() => setSidePanel('sessions')}
                      className={cn(
                        'rounded-md px-3 py-2 text-sm',
                        sidePanel === 'sessions'
                          ? 'bg-primary text-high'
                          : 'text-low hover:text-high'
                      )}
                    >
                      세션 {snapshot.sessions.length}
                    </button>
                  </div>
                  {sidePanel === 'library' && (
                    <div className="flex items-center gap-2">
                      {snapshot.room.ssh_alias && (
                        <button
                          type="button"
                          disabled={busy || !snapshot.remote.available}
                          onClick={() => void importRemoteDocuments()}
                          title="서버 .ai-room의 세션 외 Markdown 파일을 가져옵니다."
                          className="flex items-center gap-1 rounded-md border border-border px-2.5 py-1.5 text-xs hover:bg-primary disabled:opacity-40"
                        >
                          <CloudIcon className="h-3.5 w-3.5" /> 서버 문서
                          가져오기
                        </button>
                      )}
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => void createLibraryFile()}
                        className="flex items-center gap-1 rounded-md border border-border px-2.5 py-1.5 text-xs hover:bg-primary disabled:opacity-40"
                      >
                        <PlusIcon className="h-3.5 w-3.5" /> 새 문서
                      </button>
                    </div>
                  )}
                </div>
                {sidePanel === 'library' ? (
                  <>
                    <div className="max-h-48 overflow-y-auto border-b border-border p-2">
                      {snapshot.library.map((file) => (
                        <button
                          key={file.filename}
                          onClick={() => setSelectedLibraryFile(file.filename)}
                          className={cn(
                            'mb-1 w-full rounded-md p-2 text-left',
                            activeLibrary?.filename === file.filename
                              ? 'bg-primary'
                              : 'hover:bg-primary/60'
                          )}
                        >
                          <p className="flex items-center gap-2 truncate font-mono text-xs text-high">
                            <FileTextIcon className="h-4 w-4 shrink-0" />
                            {file.filename.replace('library/', '')}
                          </p>
                          <p className="mt-1 pl-6 text-xs text-low">
                            {file.source === 'both'
                              ? '로컬 + 서버'
                              : file.source}
                          </p>
                        </button>
                      ))}
                      {snapshot.library.length === 0 && (
                        <p className="p-5 text-center text-sm leading-6 text-low">
                          AI에게 작업 방식이나 규칙을 룸에 저장하라고 하면
                          여기에 문서가 나타납니다.
                        </p>
                      )}
                    </div>
                    <textarea
                      value={libraryDraft}
                      onChange={(event) => setLibraryDraft(event.target.value)}
                      disabled={
                        !activeLibrary || activeLibrary.filename === 'ROOM.md'
                      }
                      spellCheck={false}
                      placeholder="선택한 룸 문서의 내용이 여기에 표시됩니다."
                      className="min-h-0 flex-1 resize-none bg-transparent p-4 font-mono text-xs leading-5 text-normal outline-none disabled:opacity-50"
                    />
                    <div className="flex justify-between gap-2 border-t border-border p-3">
                      <button
                        type="button"
                        disabled={
                          busy ||
                          !activeLibrary ||
                          !activeLibrary.filename.startsWith('library/')
                        }
                        onClick={() => void deleteLibraryFile()}
                        className="flex items-center gap-1 rounded-md border border-red-500/40 px-3 py-1.5 text-sm text-red-400 hover:bg-red-500/10 disabled:opacity-40"
                      >
                        <TrashIcon className="h-4 w-4" /> 문서 삭제
                      </button>
                      <button
                        type="button"
                        disabled={
                          busy ||
                          !activeLibrary ||
                          activeLibrary.filename === 'ROOM.md' ||
                          libraryDraft === activeLibrary.content
                        }
                        onClick={() => void saveLibraryFile()}
                        className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-primary disabled:opacity-40"
                      >
                        문서 저장
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <div className="max-h-48 overflow-y-auto border-b border-border p-2">
                      {snapshot.sessions.map((session) => (
                        <button
                          key={session.filename}
                          onClick={() => setSelectedSession(session.filename)}
                          className={cn(
                            'mb-1 w-full rounded-md p-2 text-left',
                            activeSession?.filename === session.filename
                              ? 'bg-primary'
                              : 'hover:bg-primary/60'
                          )}
                        >
                          <p className="truncate font-mono text-xs text-high">
                            {session.filename.replace('sessions/', '')}
                          </p>
                          <p className="mt-1 text-xs text-low">
                            {session.source === 'both'
                              ? '로컬 + 서버'
                              : session.source}
                          </p>
                        </button>
                      ))}
                      {snapshot.sessions.length === 0 && (
                        <p className="p-5 text-center text-sm text-low">
                          아직 세션 기록이 없습니다.
                        </p>
                      )}
                    </div>
                    <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words p-4 font-mono text-xs leading-5 text-normal">
                      {activeSession?.content ||
                        'Claude 또는 Codex가 첫 작업을 시작하면 여기에 기록이 나타납니다.'}
                    </pre>
                  </>
                )}
              </section>
            </div>

            {snapshot.conflicts.length > 0 && (
              <section className="mt-5 rounded-lg border border-yellow-500/30 bg-yellow-500/10 p-4 text-sm text-yellow-400">
                같은 이름인데 내용이 다른 기록은 덮어쓰지 않았습니다:{' '}
                {snapshot.conflicts.join(', ')}
              </section>
            )}

            <div className="mt-8 flex justify-end border-t border-border pt-5">
              <button
                disabled={busy}
                onClick={() => void deleteRoom()}
                className="flex items-center gap-2 text-sm text-red-400 hover:text-red-300 disabled:opacity-50"
              >
                <TrashIcon className="h-4 w-4" /> 룸 등록 삭제
              </button>
            </div>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center p-8 text-center">
            <div>
              <FolderIcon
                className="mx-auto h-12 w-12 text-low"
                weight="duotone"
              />
              <h2 className="mt-4 text-lg font-medium text-high">
                프로젝트 룸을 선택하세요
              </h2>
              <p className="mt-2 text-sm text-low">
                없다면 왼쪽의 새 룸 버튼으로 로컬과 서버를 연결하세요.
              </p>
              {error && <p className="mt-4 text-sm text-red-400">{error}</p>}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}
