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
  AiRoomStorageStatus,
  SshConnectionInfo,
  SshHostsResponse,
} from 'shared/types';

import { ApiError, aiRoomsApi, sshHostsApi } from '@/shared/lib/api';
import { cn } from '@/shared/lib/utils';

type DocumentKind = 'context' | 'decisions' | 'tasks';
type RecordView = DocumentKind | 'library' | 'sessions';

const DOCUMENTS: Array<{
  kind: DocumentKind;
  label: string;
  managed: boolean;
}> = [
  { kind: 'context', label: '프로젝트 맥락', managed: false },
  { kind: 'decisions', label: '결정 기록', managed: true },
  { kind: 'tasks', label: '작업 목록', managed: true },
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

function checkpointAge(seconds: number | null) {
  if (seconds === null) return '갱신 시각을 확인할 수 없음';
  if (seconds < 60) return '방금 전';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}분 전`;
  const hours = Math.floor(minutes / 60);
  return `${hours}시간 ${minutes % 60}분 전`;
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
                : '준비됨'}
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
  const [storageStatus, setStorageStatus] =
    useState<AiRoomStorageStatus | null>(null);
  const [selectedDocument, setSelectedDocument] =
    useState<DocumentKind>('context');
  const [draft, setDraft] = useState('');
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [selectedLibraryFile, setSelectedLibraryFile] = useState<string | null>(
    null
  );
  const [libraryDraft, setLibraryDraft] = useState('');
  const [recordView, setRecordView] = useState<RecordView>('context');
  const [showCreate, setShowCreate] = useState(false);
  const [showServerManagement, setShowServerManagement] = useState(false);
  const [showConnectionEditor, setShowConnectionEditor] = useState(false);
  const [showProfileEditor, setShowProfileEditor] = useState(false);
  const [profileForm, setProfileForm] = useState({ name: '', description: '' });
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
  const [connectionForm, setConnectionForm] = useState({
    sshAlias: '',
    remoteRoot: '',
    force: false,
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

  const loadHosts = useCallback(async () => {
    const next = await sshHostsApi.list();
    setHosts(next);
    return next;
  }, []);

  useEffect(() => {
    void loadRooms()
      .catch((reason) => setError(errorMessage(reason)))
      .finally(() => setLoading(false));
    void loadHosts().catch((reason) => setError(errorMessage(reason)));
  }, [loadHosts, loadRooms]);

  const refreshSnapshot = useCallback(async (roomId: string) => {
    setError(null);
    const next = await aiRoomsApi.snapshot(roomId);
    setSnapshot(next);
    return next;
  }, []);

  useEffect(() => {
    if (!selectedRoomId) {
      setSnapshot(null);
      setStorageStatus(null);
      return;
    }
    setLoading(true);
    void Promise.all([
      refreshSnapshot(selectedRoomId),
      aiRoomsApi.storage(selectedRoomId).then(setStorageStatus),
    ])
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

  const inspectConnectionHost = async (alias: string) => {
    setConnectionForm((current) => ({
      ...current,
      sshAlias: alias,
      remoteRoot: '',
    }));
    setConnection(null);
    if (!alias) return;
    setBusy(true);
    setError(null);
    try {
      const next = await sshHostsApi.inspect(alias);
      setConnection(next);
      setConnectionForm((current) => ({
        ...current,
        remoteRoot: next.repositories[0] ?? '',
      }));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const openConnectionEditor = () => {
    if (!snapshot) return;
    void loadHosts().catch((reason) => setError(errorMessage(reason)));
    setConnectionForm({
      sshAlias: snapshot.room.ssh_alias ?? '',
      remoteRoot: snapshot.room.remote_root ?? '',
      force: false,
    });
    setConnection(null);
    setShowProfileEditor(false);
    setShowConnectionEditor(true);
  };

  const openProfileEditor = () => {
    if (!snapshot) return;
    setProfileForm({
      name: snapshot.room.name,
      description: snapshot.room.description ?? '',
    });
    setShowConnectionEditor(false);
    setShowProfileEditor(true);
  };

  const saveProfile = async (event: FormEvent) => {
    event.preventDefault();
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const next = await aiRoomsApi.updateProfile(selectedRoomId, {
        name: profileForm.name.trim(),
        description: profileForm.description.trim() || null,
      });
      setSnapshot(next);
      await loadRooms();
      setShowProfileEditor(false);
      setNotice('룸 이름과 설명을 저장하고 설명서에 반영했습니다.');
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const saveConnection = async (event: FormEvent) => {
    event.preventDefault();
    if (!selectedRoomId) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const next = await aiRoomsApi.updateConnection(selectedRoomId, {
        ssh_alias: connectionForm.sshAlias || null,
        remote_root: connectionForm.sshAlias ? connectionForm.remoteRoot : null,
        force: connectionForm.force,
      });
      setSnapshot(next);
      await loadRooms();
      setShowConnectionEditor(false);
      setNotice(
        next.room.ssh_alias
          ? '기존 서버 기록을 로컬로 회수하고 새 SSH 서버에 룸 기록과 설명서를 배치했습니다.'
          : '기존 서버 기록을 로컬로 회수하고 이 룸을 로컬 전용으로 전환했습니다.'
      );
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const disconnectServer = async (room: AiRoom) => {
    if (!room.ssh_alias) return;
    if (
      !window.confirm(
        `'${room.name}'에서 '${room.ssh_alias}' 서버 연결을 제거할까요? 로컬 룸과 지금까지 가져온 기록은 그대로 남습니다.`
      )
    )
      return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const next = await aiRoomsApi.updateConnection(room.id, {
        ssh_alias: null,
        remote_root: null,
        force: true,
      });
      if (selectedRoomId === room.id) setSnapshot(next);
      await loadRooms();
      setNotice(
        `'${room.name}'의 폐기 서버 연결을 제거했습니다. 로컬 기록은 그대로 보존됩니다.`
      );
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
      const payload = {
        name: form.name,
        description: form.description.trim() || null,
        local_root: form.localRoot,
        ssh_alias: form.sshAlias || null,
        remote_root: form.sshAlias ? form.remoteRoot : null,
        allow_existing_local_root: false,
      };
      let room: AiRoom;
      try {
        room = await aiRoomsApi.create(payload);
      } catch (reason) {
        const needsExistingFolderApproval =
          reason instanceof ApiError &&
          reason.statusCode === 409 &&
          reason.message.includes('LOCAL_ROOT_NOT_EMPTY');
        if (!needsExistingFolderApproval) throw reason;
        if (
          !window.confirm(
            '선택한 폴더에 기존 파일이 있습니다. 기존 파일은 삭제하지 않고 AI Room 기록과 설명서만 추가합니다. 이 폴더에 연결할까요?'
          )
        )
          return;
        room = await aiRoomsApi.create({
          ...payload,
          allow_existing_local_root: true,
        });
      }
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
          ? `합집합 동기화: 로컬에 ${result.copied_to_local.length}개, 서버에 ${result.copied_to_remote.length}개를 추가했습니다. ${result.conflicts.length}개 경로 충돌은 두 내용 모두 별도 기록으로 보존했습니다.`
          : `합집합 동기화 완료: 로컬에 ${result.copied_to_local.length}개, 서버에 ${result.copied_to_remote.length}개를 추가해 양쪽 세션 기록을 맞췄습니다.`
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
      setRecordView('library');
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
        '서버 설명서를 현재 규칙으로 안전하게 갱신했습니다. 진행 중 기록은 보존하며 완료 뒤에만 정리합니다.'
      );
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const saveDocument = async () => {
    if (!selectedRoomId || selectedDocument !== 'context') return;
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

  const updateSessionStatus = async (status: 'stopped' | null) => {
    if (!selectedRoomId || !activeSession) return;
    setBusy(true);
    setError(null);
    try {
      const next = await aiRoomsApi.updateSessionStatus(
        selectedRoomId,
        activeSession.filename,
        status
      );
      setSnapshot(next);
      setNotice(
        status === 'stopped'
          ? '이 세션을 중단으로 표시했습니다. 자동 작업 목록에서 제외됩니다.'
          : '세션 중단 표시를 해제했습니다.'
      );
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
      setRecordView('library');
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
      setRecordView('library');
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

  const hasUnsavedRecordEdit = Boolean(
    snapshot &&
      ((recordView === 'context' && draft !== snapshot.context) ||
        (recordView === 'library' &&
          activeLibrary &&
          libraryDraft !== activeLibrary.content))
  );

  useEffect(() => {
    if (
      !selectedRoomId ||
      showCreate ||
      showServerManagement ||
      recordView === 'context' ||
      recordView === 'library' ||
      hasUnsavedRecordEdit
    )
      return;
    const timer = window.setInterval(() => {
      if (busy || document.visibilityState === 'hidden') return;
      void aiRoomsApi
        .snapshot(selectedRoomId)
        .then(setSnapshot)
        .catch(() => undefined);
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [
    busy,
    hasUnsavedRecordEdit,
    recordView,
    selectedRoomId,
    showCreate,
    showServerManagement,
  ]);

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
            onClick={() => {
              setShowServerManagement(false);
              setShowCreate((value) => {
                if (!value) {
                  void loadHosts().catch((reason) =>
                    setError(errorMessage(reason))
                  );
                }
                return !value;
              });
            }}
            className="mt-4 flex w-full items-center justify-center gap-2 rounded-md bg-brand px-3 py-2 text-sm font-medium text-white hover:opacity-90"
          >
            <PlusIcon className="h-4 w-4" /> 새 룸
          </button>
          <button
            type="button"
            onClick={() => {
              setShowCreate(false);
              setShowServerManagement(true);
              setError(null);
            }}
            className={cn(
              'mt-2 flex w-full items-center justify-center gap-2 rounded-md border px-3 py-2 text-sm font-medium',
              showServerManagement
                ? 'border-brand bg-primary text-high'
                : 'border-border text-normal hover:bg-primary'
            )}
          >
            <HardDrivesIcon className="h-4 w-4" /> 서버 연결 관리
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {rooms.map((room) => (
            <button
              key={room.id}
              type="button"
              onClick={() => {
                setShowCreate(false);
                setShowServerManagement(false);
                setSelectedRoomId(room.id);
              }}
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
        {showServerManagement ? (
          <div className="mx-auto max-w-5xl p-8">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div>
                <h2 className="text-xl font-semibold text-high">
                  서버 연결 관리
                </h2>
                <p className="mt-2 text-sm leading-6 text-low">
                  룸에 연결했던 SSH 서버를 확인하고, 사용이 끝난 서버 연결만
                  제거합니다. 룸과 로컬 세션 기록은 삭제되지 않습니다.
                </p>
              </div>
              <span className="rounded-full bg-secondary px-3 py-1 text-xs text-low">
                {rooms.filter((room) => room.ssh_alias).length}개 서버 연결
              </span>
            </div>

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

            <div className="mt-6 space-y-3">
              {rooms
                .filter((room) => room.ssh_alias)
                .map((room) => (
                  <section
                    key={room.id}
                    className="rounded-lg border border-border bg-secondary p-5"
                  >
                    <div className="flex flex-wrap items-center justify-between gap-4">
                      <div className="min-w-0">
                        <h3 className="font-medium text-high">{room.name}</h3>
                        <p className="mt-2 break-all font-mono text-xs text-normal">
                          {room.ssh_alias}:{room.remote_root}
                        </p>
                        <p className="mt-1 break-all text-xs text-low">
                          로컬: {room.local_root}
                        </p>
                      </div>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => void disconnectServer(room)}
                        className="shrink-0 rounded-md border border-red-500/40 px-4 py-2 text-sm text-red-400 hover:bg-red-500/10 disabled:opacity-40"
                      >
                        서버 연결 제거
                      </button>
                    </div>
                  </section>
                ))}
              {rooms.every((room) => !room.ssh_alias) && (
                <div className="rounded-lg border border-dashed border-border p-10 text-center">
                  <p className="text-sm text-low">
                    현재 룸에 연결된 SSH 서버가 없습니다.
                  </p>
                </div>
              )}
            </div>

            <div className="mt-6 rounded-lg border border-border bg-secondary p-4 text-xs leading-5 text-low">
              연결 제거는 Task AI Platform의 룸 연결 정보만 정리합니다.
              Windows의 SSH 키와 <code>~/.ssh/config</code> 항목은 수정하지
              않습니다. 접속할 수 없는 폐기 서버도 강제로 연결 해제할 수
              있습니다.
            </div>
          </div>
        ) : showCreate ? (
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
                보관함, 세션 폴더를 만듭니다. 서버 기록은 준비 후 상주하며
                자동으로 삭제되지 않습니다. 동기화할 때 로컬과 서버 기록의
                합집합을 만들어 양쪽에 똑같이 보관합니다.
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
          <div className="mx-auto max-w-[1500px] p-6 lg:p-8">
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
                  onClick={openProfileEditor}
                  className="rounded-md border border-border px-3 py-2 text-sm hover:bg-secondary disabled:opacity-50"
                >
                  이름·설명 수정
                </button>
                <button
                  disabled={busy}
                  onClick={openConnectionEditor}
                  className="rounded-md border border-border px-3 py-2 text-sm hover:bg-secondary disabled:opacity-50"
                >
                  SSH 서버 변경
                </button>
                <button
                  disabled={busy}
                  onClick={() => void initializeRoom()}
                  className="rounded-md border border-border px-3 py-2 text-sm hover:bg-secondary disabled:opacity-50"
                >
                  설명서 재설치
                </button>
                {snapshot.remote.configured && (
                  <button
                    disabled={busy}
                    onClick={() => void prepareRemote()}
                    className="rounded-md border border-border px-3 py-2 text-sm hover:bg-secondary disabled:opacity-50"
                  >
                    {snapshot.remote.instruction_installed
                      ? '서버 설명서 갱신'
                      : '서버 작업 준비'}
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

            <section className="mt-6 grid gap-3 md:grid-cols-3">
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
              <div className="rounded-lg border border-border bg-primary p-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="flex min-w-0 items-center gap-2">
                    <CloudIcon className="h-5 w-5 shrink-0 text-brand" />
                    <span className="font-medium text-high">기록 보관</span>
                  </div>
                  <span className="text-xs text-low">
                    {storageStatus?.profile.mode === 'LOCAL_ONLY'
                      ? '이 PC만'
                      : storageStatus?.profile.mode === 'TASK_AI_CLOUD'
                        ? 'Task AI Cloud'
                        : storageStatus?.profile.mode === 'PERSONAL_HUB'
                          ? '개인 허브'
                          : '확인 중'}
                  </span>
                </div>
                <p className="mt-2 text-xs leading-5 text-low">
                  {storageStatus
                    ? '현재 기록은 이 기기에 보관됩니다. Cloud와 개인 허브는 실제 저장소가 준비된 뒤에만 선택할 수 있습니다.'
                    : '기록 보관 상태를 확인하고 있습니다.'}
                </p>
              </div>
            </section>

            <section
              className={cn(
                'mt-3 flex flex-wrap items-center justify-between gap-3 rounded-lg border p-4',
                snapshot.checkpoint_health.unrecorded_activity
                  ? 'border-fuchsia-500/40 bg-fuchsia-500/10'
                  : snapshot.checkpoint_health.status === 'overdue'
                    ? 'border-yellow-500/40 bg-yellow-500/10'
                    : 'border-border bg-secondary'
              )}
            >
              <div className="flex min-w-0 items-start gap-3">
                {snapshot.checkpoint_health.unrecorded_activity ? (
                  <WarningCircleIcon className="mt-0.5 h-5 w-5 shrink-0 text-fuchsia-500" />
                ) : snapshot.checkpoint_health.status === 'overdue' ? (
                  <WarningCircleIcon className="mt-0.5 h-5 w-5 shrink-0 text-yellow-500" />
                ) : (
                  <CheckCircleIcon className="mt-0.5 h-5 w-5 shrink-0 text-green-500" />
                )}
                <div className="min-w-0">
                  <h3 className="font-medium text-high">
                    {snapshot.checkpoint_health.unrecorded_activity
                      ? '작업 활동은 감지되는데 세션 기록이 갱신되지 않고 있습니다'
                      : snapshot.checkpoint_health.status === 'overdue'
                        ? '세션 체크포인트가 늦고 있습니다'
                        : snapshot.checkpoint_health.status === 'healthy'
                          ? '세션 기록이 정상적으로 갱신되고 있습니다'
                          : snapshot.checkpoint_health.status === 'unknown'
                            ? '활성 세션의 갱신 시각을 확인할 수 없습니다'
                            : '현재 진행 중으로 표시된 세션이 없습니다'}
                  </h3>
                  <p className="mt-1 break-words text-xs leading-5 text-low">
                    {snapshot.checkpoint_health.latest_session
                      ? `${snapshot.checkpoint_health.latest_session.replace('sessions/', '')} · ${checkpointAge(snapshot.checkpoint_health.latest_checkpoint_age_seconds)}`
                      : 'Claude 또는 Codex가 작업을 시작하면 대화 전용 세션 폴더와 첫 체크포인트를 만들어야 합니다.'}
                    {snapshot.checkpoint_health.status === 'overdue' &&
                      snapshot.checkpoint_health.overdue_sessions > 0 &&
                      ` · ${snapshot.checkpoint_health.overdue_sessions}개 기록 지연`}
                    {snapshot.checkpoint_health.unrecorded_activity &&
                      snapshot.checkpoint_health.local_activity_age_seconds !=
                        null &&
                      ` · 로컬 작업 활동 ${checkpointAge(snapshot.checkpoint_health.local_activity_age_seconds)}`}
                    {snapshot.checkpoint_health.unrecorded_activity &&
                      snapshot.checkpoint_health.remote_activity_age_seconds !=
                        null &&
                      ` · 서버 작업 활동 ${checkpointAge(snapshot.checkpoint_health.remote_activity_age_seconds)}`}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={() => setRecordView('sessions')}
                className="rounded-md border border-border bg-primary px-3 py-1.5 text-sm text-high hover:border-brand"
              >
                세션 기록 확인
              </button>
            </section>

            {showProfileEditor && (
              <form
                onSubmit={(event) => void saveProfile(event)}
                className="mt-4 rounded-lg border border-brand/40 bg-secondary p-5"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h3 className="font-medium text-high">룸 이름·설명 수정</h3>
                    <p className="mt-1 text-sm leading-6 text-low">
                      저장하면 로컬 설명서(ROOM.md)와 연결된 서버 설명서에도 새
                      이름이 반영됩니다.
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => setShowProfileEditor(false)}
                    className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-primary"
                  >
                    닫기
                  </button>
                </div>
                <label className="mt-4 block text-sm text-normal">
                  룸 이름
                  <input
                    value={profileForm.name}
                    onChange={(event) =>
                      setProfileForm((current) => ({
                        ...current,
                        name: event.target.value,
                      }))
                    }
                    maxLength={200}
                    className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 text-sm outline-none focus:border-brand"
                  />
                </label>
                <label className="mt-3 block text-sm text-normal">
                  룸 설명
                  <textarea
                    value={profileForm.description}
                    onChange={(event) =>
                      setProfileForm((current) => ({
                        ...current,
                        description: event.target.value,
                      }))
                    }
                    maxLength={2000}
                    rows={3}
                    className="mt-2 w-full resize-none rounded-md border border-border bg-primary px-3 py-2 text-sm outline-none focus:border-brand"
                  />
                </label>
                <div className="mt-4 flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => setShowProfileEditor(false)}
                    disabled={busy}
                    className="rounded-md border border-border px-4 py-2 text-sm hover:bg-primary disabled:opacity-50"
                  >
                    취소
                  </button>
                  <button
                    type="submit"
                    disabled={busy || !profileForm.name.trim()}
                    className="rounded-md bg-brand px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
                  >
                    {busy ? '저장 중…' : '저장'}
                  </button>
                </div>
              </form>
            )}

            {showConnectionEditor && (
              <form
                onSubmit={(event) => void saveConnection(event)}
                className="mt-4 rounded-lg border border-brand/40 bg-secondary p-5"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h3 className="font-medium text-high">SSH 서버 변경</h3>
                    <p className="mt-1 text-sm leading-6 text-low">
                      기존 서버의 최신 세션을 먼저 로컬로 가져온 뒤 새 서버에 룸
                      기록과 설명서를 자동으로 설치합니다.
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => setShowConnectionEditor(false)}
                    className="text-xs text-low hover:text-high"
                  >
                    닫기
                  </button>
                </div>
                <div className="mt-4 grid gap-4 sm:grid-cols-2">
                  <label className="block text-sm text-high">
                    SSH 서버
                    <select
                      value={connectionForm.sshAlias}
                      onChange={(event) =>
                        void inspectConnectionHost(event.target.value)
                      }
                      className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 outline-none focus:border-brand"
                    >
                      <option value="">로컬 전용으로 변경</option>
                      {hosts?.hosts.map((host) => (
                        <option key={host.alias} value={host.alias}>
                          {host.alias} ({host.hostname})
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="block text-sm text-high">
                    새 서버 프로젝트 경로
                    <input
                      list="connection-remote-repositories"
                      required={Boolean(connectionForm.sshAlias)}
                      disabled={!connectionForm.sshAlias}
                      value={connectionForm.remoteRoot}
                      onChange={(event) =>
                        setConnectionForm((current) => ({
                          ...current,
                          remoteRoot: event.target.value,
                        }))
                      }
                      className="mt-2 w-full rounded-md border border-border bg-primary px-3 py-2 font-mono text-sm outline-none disabled:opacity-50 focus:border-brand"
                      placeholder="/home/user/project"
                    />
                    <datalist id="connection-remote-repositories">
                      {connection?.repositories.map((repository) => (
                        <option key={repository} value={repository} />
                      ))}
                    </datalist>
                  </label>
                </div>
                {snapshot.remote.configured && (
                  <label className="mt-4 flex items-start gap-2 text-xs text-low">
                    <input
                      type="checkbox"
                      checked={connectionForm.force}
                      onChange={(event) =>
                        setConnectionForm((current) => ({
                          ...current,
                          force: event.target.checked,
                        }))
                      }
                      className="mt-0.5"
                    />
                    기존 서버에 접속할 수 없어도 로컬에 남은 기록만으로 강제
                    전환
                  </label>
                )}
                <div className="mt-5 flex justify-end gap-2">
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => setShowConnectionEditor(false)}
                    className="rounded-md border border-border px-4 py-2 text-sm hover:bg-primary disabled:opacity-50"
                  >
                    취소
                  </button>
                  <button
                    disabled={busy}
                    className="rounded-md bg-brand px-4 py-2 text-sm font-medium text-white disabled:opacity-50"
                  >
                    {busy ? '서버 전환 중…' : '기록을 이어서 서버 변경'}
                  </button>
                </div>
              </form>
            )}

            <section className="mt-6 rounded-lg border border-border bg-secondary p-5">
              <div className="flex items-start gap-3">
                <CloudIcon className="mt-0.5 h-5 w-5 shrink-0 text-brand" />
                <div>
                  <h3 className="font-medium text-high">사용 방법</h3>
                  <p className="mt-1 text-sm leading-6 text-low">
                    이 앱에서 채팅하지 않습니다. 로컬에서는 Claude 또는 Codex를
                    바로 실행하세요. 서버에서 작업할 때는 먼저{' '}
                    <strong>서버 작업 준비</strong>을 누른 뒤 서버 프로젝트
                    루트에서 실행합니다. 작업 중 체크포인트는 서버에서 삭제하지
                    않고 자동으로 로컬에 복사됩니다.{' '}
                    {snapshot.local_summary_enabled ? (
                      <>
                        2분간 기록이 안정되면 작업 목록과 결정 기록을 로컬 AI가
                        갱신합니다.
                      </>
                    ) : (
                      <>
                        작업 목록과 결정 기록의 로컬 AI 자동 갱신은 꺼져
                        있습니다. 이 기능은 컴퓨터의 GPU를 사용하므로, 켜려면
                        환경변수 <code>TASK_AI_LOCAL_SUMMARY=1</code>을 설정하고
                        앱을 다시 시작하세요.
                      </>
                    )}{' '}
                    AI에게 작업 방식, 규칙, 절차를 룸에 저장하라고 하면{' '}
                    <code>.ai-room/library</code>에 문서로 남고 오른쪽의{' '}
                    <strong>룸 문서</strong> 목록에 나타납니다. 서버의{' '}
                    <code>.ai-room</code> 기록은 자동으로 삭제되지 않고
                    상주합니다.
                  </p>
                </div>
              </div>
            </section>

            <section className="mt-6 overflow-hidden rounded-lg border border-border bg-secondary">
              <nav className="flex flex-wrap gap-1 border-b border-border p-2">
                {DOCUMENTS.map((document) => (
                  <button
                    key={document.kind}
                    type="button"
                    onClick={() => {
                      setSelectedDocument(document.kind);
                      setRecordView(document.kind);
                    }}
                    className={cn(
                      'rounded-md px-4 py-2.5 text-sm font-medium',
                      recordView === document.kind
                        ? 'bg-primary text-high shadow-sm'
                        : 'text-low hover:bg-primary/60 hover:text-high'
                    )}
                  >
                    {document.label}
                    {document.managed && !snapshot.local_summary_enabled && (
                      <span className="ml-2 rounded-full bg-panel px-2 py-0.5 text-xs text-low">
                        자동 갱신 꺼짐
                      </span>
                    )}
                  </button>
                ))}
                <button
                  type="button"
                  onClick={() => setRecordView('library')}
                  className={cn(
                    'rounded-md px-4 py-2.5 text-sm font-medium',
                    recordView === 'library'
                      ? 'bg-primary text-high shadow-sm'
                      : 'text-low hover:bg-primary/60 hover:text-high'
                  )}
                >
                  룸 문서
                  <span className="ml-2 rounded-full bg-panel px-2 py-0.5 text-xs text-low">
                    {snapshot.library.length}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => setRecordView('sessions')}
                  className={cn(
                    'rounded-md px-4 py-2.5 text-sm font-medium',
                    recordView === 'sessions'
                      ? 'bg-primary text-high shadow-sm'
                      : 'text-low hover:bg-primary/60 hover:text-high'
                  )}
                >
                  세션 기록
                  <span className="ml-2 rounded-full bg-panel px-2 py-0.5 text-xs text-low">
                    {snapshot.sessions.length}
                  </span>
                  {snapshot.checkpoint_health.status === 'overdue' && (
                    <span className="ml-1 text-yellow-500">●</span>
                  )}
                </button>
              </nav>

              {recordView === 'context' ||
              recordView === 'decisions' ||
              recordView === 'tasks' ? (
                <div className="flex min-h-[640px] flex-col">
                  <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-5 py-4">
                    <div>
                      <h3 className="font-medium text-high">
                        {
                          DOCUMENTS.find(
                            (document) => document.kind === selectedDocument
                          )?.label
                        }
                      </h3>
                      <p className="mt-1 text-xs text-low">
                        {selectedDocument === 'context'
                          ? '프로젝트의 변하지 않는 배경과 목표를 직접 관리합니다.'
                          : snapshot.local_summary_enabled
                            ? '안정된 세션 기록을 바탕으로 로컬 AI가 자동 정리합니다.'
                            : '자동 갱신이 꺼져 있습니다. 아래 내용은 마지막으로 갱신된 시점 그대로이며 최신이 아닐 수 있습니다.'}
                      </p>
                    </div>
                    <button
                      disabled={
                        busy ||
                        selectedDocument !== 'context' ||
                        draft === documentContent(snapshot, selectedDocument)
                      }
                      onClick={() => void saveDocument()}
                      className="rounded-md border border-border px-4 py-2 text-sm hover:bg-primary disabled:opacity-40"
                    >
                      {selectedDocument === 'context'
                        ? '프로젝트 맥락 저장'
                        : 'AI 자동 관리'}
                    </button>
                  </div>
                  <textarea
                    value={draft}
                    onChange={(event) => setDraft(event.target.value)}
                    readOnly={selectedDocument !== 'context'}
                    spellCheck={false}
                    className="min-h-[560px] flex-1 resize-none bg-transparent p-6 font-mono text-sm leading-7 text-normal outline-none read-only:opacity-90"
                  />
                </div>
              ) : recordView === 'library' ? (
                <div className="grid min-h-[660px] lg:grid-cols-[minmax(280px,0.55fr)_minmax(0,1.45fr)]">
                  <aside className="flex min-h-0 flex-col border-b border-border lg:border-b-0 lg:border-r">
                    <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border p-3">
                      <div>
                        <h3 className="font-medium text-high">룸 문서 목록</h3>
                        <p className="mt-1 text-xs text-low">
                          규칙·절차·참고 자료
                        </p>
                      </div>
                      <div className="flex items-center gap-2">
                        {snapshot.room.ssh_alias && (
                          <button
                            type="button"
                            disabled={busy || !snapshot.remote.available}
                            onClick={() => void importRemoteDocuments()}
                            title="서버 .ai-room의 세션 외 Markdown 파일을 가져옵니다."
                            className="flex items-center gap-1 rounded-md border border-border px-2.5 py-1.5 text-xs hover:bg-primary disabled:opacity-40"
                          >
                            <CloudIcon className="h-3.5 w-3.5" /> 서버 가져오기
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
                    </div>
                    <div className="max-h-72 flex-1 overflow-y-auto p-2 lg:max-h-none">
                      {snapshot.library.map((file) => (
                        <button
                          key={file.filename}
                          onClick={() => setSelectedLibraryFile(file.filename)}
                          className={cn(
                            'mb-1 w-full rounded-md p-3 text-left',
                            activeLibrary?.filename === file.filename
                              ? 'bg-primary'
                              : 'hover:bg-primary/60'
                          )}
                        >
                          <p className="flex items-center gap-2 truncate font-mono text-sm text-high">
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
                        <p className="p-6 text-center text-sm leading-6 text-low">
                          AI에게 작업 방식이나 규칙을 룸에 저장하라고 하면
                          여기에 문서가 나타납니다.
                        </p>
                      )}
                    </div>
                  </aside>
                  <div className="flex min-h-[520px] min-w-0 flex-col">
                    <div className="border-b border-border px-5 py-4">
                      <h3 className="truncate font-mono text-sm text-high">
                        {activeLibrary?.filename.replace('library/', '') ||
                          '문서를 선택하세요'}
                      </h3>
                      <p className="mt-1 text-xs text-low">
                        {activeLibrary
                          ? activeLibrary.source === 'both'
                            ? '로컬 + 서버'
                            : activeLibrary.source
                          : '목록에서 확인할 문서를 선택하세요.'}
                      </p>
                    </div>
                    <textarea
                      value={libraryDraft}
                      onChange={(event) => setLibraryDraft(event.target.value)}
                      disabled={
                        !activeLibrary ||
                        activeLibrary.filename === 'ROOM.md' ||
                        activeLibrary.filename === 'decisions.md' ||
                        activeLibrary.filename === 'tasks.md'
                      }
                      spellCheck={false}
                      placeholder="선택한 룸 문서의 내용이 여기에 표시됩니다."
                      className="min-h-[440px] flex-1 resize-none bg-transparent p-6 font-mono text-sm leading-7 text-normal outline-none disabled:opacity-60"
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
                          activeLibrary.filename === 'decisions.md' ||
                          activeLibrary.filename === 'tasks.md' ||
                          libraryDraft === activeLibrary.content
                        }
                        onClick={() => void saveLibraryFile()}
                        className="rounded-md border border-border px-4 py-1.5 text-sm hover:bg-primary disabled:opacity-40"
                      >
                        문서 저장
                      </button>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="grid min-h-[660px] lg:grid-cols-[minmax(320px,0.65fr)_minmax(0,1.35fr)]">
                  <aside className="flex min-h-0 flex-col border-b border-border lg:border-b-0 lg:border-r">
                    <div className="border-b border-border px-4 py-4">
                      <h3 className="font-medium text-high">세션 기록 목록</h3>
                      <p className="mt-1 text-xs text-low">
                        대화창별 폴더에 누적된 불변 체크포인트
                      </p>
                    </div>
                    <div className="max-h-72 flex-1 overflow-y-auto p-2 lg:max-h-none">
                      {snapshot.sessions.map((session) => (
                        <button
                          key={session.filename}
                          onClick={() => setSelectedSession(session.filename)}
                          className={cn(
                            'mb-1 w-full rounded-md p-3 text-left',
                            activeSession?.filename === session.filename
                              ? 'bg-primary'
                              : 'hover:bg-primary/60'
                          )}
                        >
                          <p className="break-words font-mono text-xs leading-5 text-high">
                            {session.filename.replace('sessions/', '')}
                          </p>
                          <p className="mt-1 text-xs text-low">
                            {!session.filename.endsWith('.md') &&
                              `대화 폴더 · ${(session.content.match(/<!-- AI Room checkpoint:/g) || []).length}개 기록 · `}
                            {session.source === 'both'
                              ? '로컬 + 서버'
                              : session.source === 'conflict'
                                ? '충돌 · 로컬 원본 표시'
                                : session.source === 'mixed'
                                  ? '로컬·서버 혼합'
                                  : session.source}
                            {snapshot.session_overrides[session.filename] ===
                              'stopped' && ' · ⛔ 중단'}
                          </p>
                        </button>
                      ))}
                      {snapshot.sessions.length === 0 && (
                        <p className="p-6 text-center text-sm text-low">
                          아직 세션 기록이 없습니다.
                        </p>
                      )}
                    </div>
                  </aside>
                  <div className="flex min-h-[520px] min-w-0 flex-col">
                    <div className="border-b border-border px-5 py-4">
                      <h3 className="break-words font-mono text-sm text-high">
                        {activeSession?.filename.replace('sessions/', '') ||
                          '세션을 선택하세요'}
                      </h3>
                      <p className="mt-1 text-xs text-low">
                        각 구분선은 새로 추가된 체크포인트이며 기존 기록은
                        수정하지 않습니다.
                      </p>
                    </div>
                    <pre className="min-h-[440px] flex-1 overflow-auto whitespace-pre-wrap break-words p-6 font-mono text-sm leading-7 text-normal">
                      {activeSession?.content ||
                        'Claude 또는 Codex가 첫 작업을 시작하면 여기에 기록이 나타납니다.'}
                    </pre>
                    {activeSession && (
                      <div className="flex justify-end border-t border-border p-3">
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() =>
                            void updateSessionStatus(
                              snapshot.session_overrides[
                                activeSession.filename
                              ] === 'stopped'
                                ? null
                                : 'stopped'
                            )
                          }
                          className="rounded-md border border-border px-4 py-1.5 text-sm hover:bg-primary disabled:opacity-40"
                        >
                          {snapshot.session_overrides[
                            activeSession.filename
                          ] === 'stopped'
                            ? '중단 표시 해제'
                            : '강제 종료·중단 처리'}
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              )}
            </section>

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
