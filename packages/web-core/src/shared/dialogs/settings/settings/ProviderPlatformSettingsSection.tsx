import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import {
  CheckCircleIcon,
  PlayIcon,
  PlusIcon,
  ShieldWarningIcon,
  SpinnerGapIcon,
} from '@phosphor-icons/react';
import type { CreateAssignment, ProviderPlatformSnapshot } from 'shared/types';
import { providerPlatformApi } from '@/shared/lib/api';

const inputClass =
  'w-full px-base py-2 bg-secondary rounded border text-base text-normal placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand';
const buttonClass =
  'inline-flex items-center justify-center gap-2 px-base py-2 rounded border bg-secondary text-normal hover:text-high disabled:opacity-50 disabled:cursor-not-allowed';
const primaryButtonClass = `${buttonClass} bg-brand text-white border-brand hover:text-white`;

function splitIds(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean);
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-sm text-low">
      <span>{label}</span>
      {children}
    </label>
  );
}

function Panel({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded border bg-panel p-base space-y-base">
      <header>
        <h3 className="text-lg font-medium text-high">{title}</h3>
        <p className="mt-1 text-sm text-low">{description}</p>
      </header>
      {children}
    </section>
  );
}

function EmptyOption({ children }: { children: ReactNode }) {
  return <option value="">{children}</option>;
}

export function ProviderPlatformSettingsSection() {
  const [snapshot, setSnapshot] = useState<ProviderPlatformSnapshot | null>(
    null
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [pluginDirectory, setPluginDirectory] = useState(
    'C:\\AI-Workspace\\task-ai-platform\\provider-plugins\\mock'
  );

  const [accountPluginId, setAccountPluginId] = useState('');
  const [accountName, setAccountName] = useState('');
  const [accountType, setAccountType] = useState('local_runtime');

  const [modelAccountId, setModelAccountId] = useState('');
  const [modelKey, setModelKey] = useState('');
  const [modelName, setModelName] = useState('');
  const [modelCapabilities, setModelCapabilities] = useState('chat, streaming');

  const [runtimeAccountId, setRuntimeAccountId] = useState('');
  const [runtimeName, setRuntimeName] = useState('');
  const [runtimeType, setRuntimeType] = useState('local_runtime');
  const [runtimeExecutable, setRuntimeExecutable] = useState('');
  const [runtimeEndpoint, setRuntimeEndpoint] = useState('');

  const [policyName, setPolicyName] = useState('');
  const [policyPermissions, setPolicyPermissions] = useState('');

  const [agentName, setAgentName] = useState('');
  const [agentRole, setAgentRole] = useState('backend-developer');
  const [agentCapabilities, setAgentCapabilities] = useState('');
  const [agentInstructions, setAgentInstructions] = useState('');
  const [agentPolicyId, setAgentPolicyId] = useState('');

  const [taskId, setTaskId] = useState('');
  const [assignmentPluginId, setAssignmentPluginId] = useState('');
  const [assignmentAccountId, setAssignmentAccountId] = useState('');
  const [assignmentModelId, setAssignmentModelId] = useState('');
  const [assignmentRuntimeId, setAssignmentRuntimeId] = useState('');
  const [assignmentAgentId, setAssignmentAgentId] = useState('');
  const [assignmentPolicyId, setAssignmentPolicyId] = useState('');
  const [requiredCapabilities, setRequiredCapabilities] = useState(
    'filesystem_read, filesystem_write, command_execution, code_edit'
  );
  const [validation, setValidation] = useState<Awaited<
    ReturnType<typeof providerPlatformApi.validateAssignment>
  > | null>(null);

  const refresh = useCallback(async () => {
    setSnapshot(await providerPlatformApi.snapshot());
  }, []);

  useEffect(() => {
    void refresh().catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [refresh]);

  useEffect(() => {
    if (!snapshot) return;
    setAccountPluginId((current) => current || snapshot.plugins[0]?.id || '');
    setModelAccountId((current) => current || snapshot.accounts[0]?.id || '');
    setRuntimeAccountId((current) => current || snapshot.accounts[0]?.id || '');
    setAgentPolicyId(
      (current) => current || snapshot.permission_policies[0]?.id || ''
    );
    setAssignmentPluginId(
      (current) => current || snapshot.plugins[0]?.id || ''
    );
    setAssignmentAgentId(
      (current) => current || snapshot.agent_profiles[0]?.id || ''
    );
    setAssignmentPolicyId(
      (current) => current || snapshot.permission_policies[0]?.id || ''
    );
  }, [snapshot]);

  const assignmentAccounts = useMemo(
    () =>
      snapshot?.accounts.filter(
        (account) => account.provider_plugin_id === assignmentPluginId
      ) ?? [],
    [assignmentPluginId, snapshot]
  );
  const assignmentModels = useMemo(
    () =>
      snapshot?.models.filter(
        (model) => model.provider_account_id === assignmentAccountId
      ) ?? [],
    [assignmentAccountId, snapshot]
  );
  const assignmentRuntimes = useMemo(
    () =>
      snapshot?.runtimes.filter(
        (runtime) => runtime.provider_account_id === assignmentAccountId
      ) ?? [],
    [assignmentAccountId, snapshot]
  );

  useEffect(() => {
    if (!assignmentAccounts.some((item) => item.id === assignmentAccountId)) {
      setAssignmentAccountId(assignmentAccounts[0]?.id ?? '');
    }
  }, [assignmentAccountId, assignmentAccounts]);

  useEffect(() => {
    if (!assignmentModels.some((item) => item.id === assignmentModelId)) {
      setAssignmentModelId(assignmentModels[0]?.id ?? '');
    }
    if (!assignmentRuntimes.some((item) => item.id === assignmentRuntimeId)) {
      setAssignmentRuntimeId(assignmentRuntimes[0]?.id ?? '');
    }
  }, [
    assignmentModelId,
    assignmentModels,
    assignmentRuntimeId,
    assignmentRuntimes,
  ]);

  const run = useCallback(
    async (message: string, action: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      setNotice(null);
      try {
        await action();
        await refresh();
        setNotice(message);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setBusy(false);
      }
    },
    [refresh]
  );

  const buildAssignment = useCallback((): CreateAssignment => {
    if (
      !taskId ||
      !assignmentPluginId ||
      !assignmentAccountId ||
      !assignmentRuntimeId ||
      !assignmentAgentId
    ) {
      throw new Error('Task와 필수 Assignment 항목을 모두 선택하세요.');
    }
    return {
      task_id: taskId,
      provider_plugin_id: assignmentPluginId,
      provider_account_id: assignmentAccountId,
      model_definition_id: assignmentModelId || null,
      runtime_profile_id: assignmentRuntimeId,
      agent_profile_id: assignmentAgentId,
      permission_policy_id: assignmentPolicyId || null,
      required_capabilities: splitIds(requiredCapabilities),
      assigned_by: 'user',
      handoff_from_assignment_id: null,
      handoff_reason: null,
    };
  }, [
    assignmentAccountId,
    assignmentAgentId,
    assignmentModelId,
    assignmentPluginId,
    assignmentPolicyId,
    assignmentRuntimeId,
    requiredCapabilities,
    taskId,
  ]);

  const validate = async () => {
    setBusy(true);
    setError(null);
    try {
      setValidation(
        await providerPlatformApi.validateAssignment(buildAssignment())
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setValidation(null);
    } finally {
      setBusy(false);
    }
  };

  if (!snapshot) {
    return (
      <div className="flex items-center gap-2 text-normal">
        <SpinnerGapIcon className="animate-spin" /> 범용 AI 플랫폼 로딩 중...
      </div>
    );
  }

  const stats = [
    ['Provider Plugin', snapshot.plugins.length],
    ['Account', snapshot.accounts.length],
    ['Model', snapshot.models.length],
    ['Runtime', snapshot.runtimes.length],
    ['Agent Profile', snapshot.agent_profiles.length],
    ['Assignment', snapshot.assignments.length],
  ] as const;

  return (
    <div className="space-y-base pb-double">
      <div>
        <h2 className="text-xl font-semibold text-high">AI Provider 플랫폼</h2>
        <p className="mt-1 text-sm text-low">
          공급자 이름이 아닌 Plugin manifest와 Capability를 기준으로 AI 실행
          자원을 관리합니다.
        </p>
      </div>

      <div className="grid grid-cols-2 lg:grid-cols-3 gap-half">
        {stats.map(([label, value]) => (
          <div key={label} className="rounded border bg-secondary p-base">
            <div className="text-xs uppercase tracking-wide text-low">
              {label}
            </div>
            <div className="mt-1 text-xl font-semibold text-high">{value}</div>
          </div>
        ))}
      </div>

      {error && (
        <div className="rounded border border-error/50 bg-error/10 p-base text-sm text-error">
          {error}
        </div>
      )}
      {notice && (
        <div className="rounded border border-success/50 bg-success/10 p-base text-sm text-success">
          {notice}
        </div>
      )}

      <Panel
        title="1. Provider Plugin 설치"
        description="provider-plugin.json을 포함한 로컬 폴더를 검증하고 등록합니다."
      >
        <div className="flex gap-half">
          <input
            className={inputClass}
            value={pluginDirectory}
            onChange={(event) => setPluginDirectory(event.target.value)}
            placeholder="Plugin 폴더 절대 경로"
          />
          <button
            className={primaryButtonClass}
            disabled={busy || !pluginDirectory}
            onClick={() =>
              void run('Provider Plugin을 설치했습니다.', () =>
                providerPlatformApi.installPlugin({
                  directory: pluginDirectory,
                })
              )
            }
          >
            <PlusIcon /> 설치
          </button>
        </div>
        <div className="space-y-half">
          {snapshot.plugins.map((plugin) => (
            <div
              key={plugin.id}
              className="flex items-start justify-between rounded bg-secondary p-base"
            >
              <div>
                <div className="text-base text-high">{plugin.display_name}</div>
                <div className="text-xs text-low">
                  {plugin.plugin_key} · v{plugin.version}
                </div>
                <div className="mt-2 flex flex-wrap gap-1">
                  {plugin.capabilities.map((capability) => (
                    <span
                      key={capability}
                      className="rounded bg-brand/10 px-2 py-1 text-xs text-brand"
                    >
                      {capability}
                    </span>
                  ))}
                </div>
              </div>
              <span className="text-xs text-success">{plugin.status}</span>
            </div>
          ))}
        </div>
      </Panel>

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-base">
        <Panel
          title="2. Account 연결"
          description="API Key, OAuth, 로컬 서버, CLI 프로필 등을 하나의 연결로 취급합니다."
        >
          <Field label="Provider Plugin">
            <select
              className={inputClass}
              value={accountPluginId}
              onChange={(event) => setAccountPluginId(event.target.value)}
            >
              <EmptyOption>Plugin 선택</EmptyOption>
              {snapshot.plugins.map((plugin) => (
                <option key={plugin.id} value={plugin.id}>
                  {plugin.display_name}
                </option>
              ))}
            </select>
          </Field>
          <div className="grid grid-cols-2 gap-half">
            <Field label="표시 이름">
              <input
                className={inputClass}
                value={accountName}
                onChange={(event) => setAccountName(event.target.value)}
                placeholder="예: 로컬 개발 연결"
              />
            </Field>
            <Field label="연결 유형">
              <input
                className={inputClass}
                value={accountType}
                onChange={(event) => setAccountType(event.target.value)}
              />
            </Field>
          </div>
          <button
            className={buttonClass}
            disabled={busy || !accountPluginId || !accountName}
            onClick={() =>
              void run('Account를 추가했습니다.', () =>
                providerPlatformApi.createAccount({
                  provider_plugin_id: accountPluginId,
                  display_name: accountName,
                  account_type: accountType,
                  credential_reference: null,
                  configuration_json: {},
                })
              )
            }
          >
            <PlusIcon /> Account 추가
          </button>
        </Panel>

        <Panel
          title="3. Model 정의"
          description="Provider 자동 탐색 결과 또는 사용자 정의 모델을 등록합니다."
        >
          <Field label="Account">
            <select
              className={inputClass}
              value={modelAccountId}
              onChange={(event) => setModelAccountId(event.target.value)}
            >
              <EmptyOption>Account 선택</EmptyOption>
              {snapshot.accounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.display_name}
                </option>
              ))}
            </select>
          </Field>
          <div className="grid grid-cols-2 gap-half">
            <Field label="Provider Model Key">
              <input
                className={inputClass}
                value={modelKey}
                onChange={(event) => setModelKey(event.target.value)}
                placeholder="model-key"
              />
            </Field>
            <Field label="표시 이름">
              <input
                className={inputClass}
                value={modelName}
                onChange={(event) => setModelName(event.target.value)}
                placeholder="모델 이름"
              />
            </Field>
          </div>
          <Field label="Capability (쉼표 구분)">
            <input
              className={inputClass}
              value={modelCapabilities}
              onChange={(event) => setModelCapabilities(event.target.value)}
            />
          </Field>
          <button
            className={buttonClass}
            disabled={busy || !modelAccountId || !modelKey || !modelName}
            onClick={() =>
              void run('Model을 추가했습니다.', () =>
                providerPlatformApi.createModel({
                  provider_account_id: modelAccountId,
                  provider_model_key: modelKey,
                  display_name: modelName,
                  description: null,
                  context_window: null,
                  input_modalities: ['text'],
                  output_modalities: ['text'],
                  capabilities: splitIds(modelCapabilities),
                  pricing_json: {},
                  discovered_automatically: false,
                })
              )
            }
          >
            <PlusIcon /> Model 추가
          </button>
        </Panel>

        <Panel
          title="4. Runtime Profile"
          description="AI가 실제로 실행되는 CLI, API, 원격 또는 앱 환경입니다."
        >
          <Field label="Account">
            <select
              className={inputClass}
              value={runtimeAccountId}
              onChange={(event) => setRuntimeAccountId(event.target.value)}
            >
              <EmptyOption>Account 선택</EmptyOption>
              {snapshot.accounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.display_name}
                </option>
              ))}
            </select>
          </Field>
          <div className="grid grid-cols-2 gap-half">
            <Field label="이름">
              <input
                className={inputClass}
                value={runtimeName}
                onChange={(event) => setRuntimeName(event.target.value)}
                placeholder="Local CLI"
              />
            </Field>
            <Field label="Runtime Type">
              <input
                className={inputClass}
                value={runtimeType}
                onChange={(event) => setRuntimeType(event.target.value)}
              />
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-half">
            <Field label="실행 파일 (선택)">
              <input
                className={inputClass}
                value={runtimeExecutable}
                onChange={(event) => setRuntimeExecutable(event.target.value)}
              />
            </Field>
            <Field label="Endpoint (선택)">
              <input
                className={inputClass}
                value={runtimeEndpoint}
                onChange={(event) => setRuntimeEndpoint(event.target.value)}
              />
            </Field>
          </div>
          <button
            className={buttonClass}
            disabled={busy || !runtimeAccountId || !runtimeName}
            onClick={() =>
              void run('Runtime을 추가했습니다.', () =>
                providerPlatformApi.createRuntime({
                  provider_account_id: runtimeAccountId,
                  name: runtimeName,
                  runtime_type: runtimeType,
                  executable_path: runtimeExecutable || null,
                  endpoint: runtimeEndpoint || null,
                  remote_connection_id: null,
                  working_directory_policy: 'workspace',
                  environment_reference: null,
                  configuration_json: {},
                })
              )
            }
          >
            <PlusIcon /> Runtime 추가
          </button>
        </Panel>

        <Panel
          title="5. 권한 정책"
          description="Plugin이 별도 프로세스에서 실행되기 전에 명시적으로 승인할 권한입니다."
        >
          <Field label="정책 이름">
            <input
              className={inputClass}
              value={policyName}
              onChange={(event) => setPolicyName(event.target.value)}
              placeholder="Local Development"
            />
          </Field>
          <Field label="승인 권한 (쉼표 구분)">
            <input
              className={inputClass}
              value={policyPermissions}
              onChange={(event) => setPolicyPermissions(event.target.value)}
              placeholder="filesystem_read, process_execution"
            />
          </Field>
          <button
            className={buttonClass}
            disabled={busy || !policyName}
            onClick={() =>
              void run('권한 정책을 추가했습니다.', () =>
                providerPlatformApi.createPermissionPolicy({
                  name: policyName,
                  description: null,
                  approved_permissions: splitIds(policyPermissions),
                  constraints_json: {},
                })
              )
            }
          >
            <PlusIcon /> 정책 추가
          </button>
        </Panel>

        <Panel
          title="6. Agent Profile"
          description="모델이 아니라 업무 역할, 지침, 도구 정책을 정의합니다."
        >
          <div className="grid grid-cols-2 gap-half">
            <Field label="이름">
              <input
                className={inputClass}
                value={agentName}
                onChange={(event) => setAgentName(event.target.value)}
                placeholder="Backend Developer"
              />
            </Field>
            <Field label="Role Key">
              <input
                className={inputClass}
                value={agentRole}
                onChange={(event) => setAgentRole(event.target.value)}
              />
            </Field>
          </div>
          <Field label="호환 Capability">
            <input
              className={inputClass}
              value={agentCapabilities}
              onChange={(event) => setAgentCapabilities(event.target.value)}
            />
          </Field>
          <Field label="System Instructions">
            <textarea
              className={inputClass}
              rows={3}
              value={agentInstructions}
              onChange={(event) => setAgentInstructions(event.target.value)}
            />
          </Field>
          <Field label="기본 권한 정책">
            <select
              className={inputClass}
              value={agentPolicyId}
              onChange={(event) => setAgentPolicyId(event.target.value)}
            >
              <EmptyOption>선택 안 함</EmptyOption>
              {snapshot.permission_policies.map((policy) => (
                <option key={policy.id} value={policy.id}>
                  {policy.name}
                </option>
              ))}
            </select>
          </Field>
          <button
            className={buttonClass}
            disabled={busy || !agentName || !agentRole}
            onClick={() =>
              void run('Agent Profile을 추가했습니다.', () =>
                providerPlatformApi.createAgentProfile({
                  name: agentName,
                  description: null,
                  role_key: agentRole,
                  system_instructions: agentInstructions,
                  compatible_capabilities: splitIds(agentCapabilities),
                  preferred_provider_plugin_id: null,
                  preferred_model_id: null,
                  allowed_tools: [],
                  denied_tools: [],
                  permission_policy_id: agentPolicyId || null,
                  context_policy_json: {},
                })
              )
            }
          >
            <PlusIcon /> Agent Profile 추가
          </button>
        </Panel>
      </div>

      <Panel
        title="7. Task Assignment Wizard"
        description="앞 단계 선택에 따라 다음 목록이 동적으로 좁혀지며, Capability와 권한을 저장 전에 검증합니다."
      >
        <Field label="Task ID">
          <input
            className={inputClass}
            value={taskId}
            onChange={(event) => setTaskId(event.target.value)}
            placeholder="로컬 또는 원격 Task ID"
          />
        </Field>
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-half">
          <Field label="1. Provider Plugin">
            <select
              className={inputClass}
              value={assignmentPluginId}
              onChange={(event) => {
                setAssignmentPluginId(event.target.value);
                setValidation(null);
              }}
            >
              <EmptyOption>선택</EmptyOption>
              {snapshot.plugins.map((plugin) => (
                <option key={plugin.id} value={plugin.id}>
                  {plugin.display_name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="2. Account">
            <select
              className={inputClass}
              value={assignmentAccountId}
              onChange={(event) => {
                setAssignmentAccountId(event.target.value);
                setValidation(null);
              }}
            >
              <EmptyOption>선택</EmptyOption>
              {assignmentAccounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.display_name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="3. Model">
            <select
              className={inputClass}
              value={assignmentModelId}
              onChange={(event) => {
                setAssignmentModelId(event.target.value);
                setValidation(null);
              }}
            >
              <EmptyOption>Model 없음</EmptyOption>
              {assignmentModels.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.display_name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="4. Runtime">
            <select
              className={inputClass}
              value={assignmentRuntimeId}
              onChange={(event) => {
                setAssignmentRuntimeId(event.target.value);
                setValidation(null);
              }}
            >
              <EmptyOption>선택</EmptyOption>
              {assignmentRuntimes.map((runtime) => (
                <option key={runtime.id} value={runtime.id}>
                  {runtime.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="5. Agent Profile">
            <select
              className={inputClass}
              value={assignmentAgentId}
              onChange={(event) => {
                setAssignmentAgentId(event.target.value);
                setValidation(null);
              }}
            >
              <EmptyOption>선택</EmptyOption>
              {snapshot.agent_profiles.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="6. Permission Policy">
            <select
              className={inputClass}
              value={assignmentPolicyId}
              onChange={(event) => {
                setAssignmentPolicyId(event.target.value);
                setValidation(null);
              }}
            >
              <EmptyOption>정책 없음</EmptyOption>
              {snapshot.permission_policies.map((policy) => (
                <option key={policy.id} value={policy.id}>
                  {policy.name}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label="Task 필수 Capability">
          <input
            className={inputClass}
            value={requiredCapabilities}
            onChange={(event) => {
              setRequiredCapabilities(event.target.value);
              setValidation(null);
            }}
          />
        </Field>
        <div className="flex gap-half">
          <button className={buttonClass} disabled={busy} onClick={validate}>
            <ShieldWarningIcon /> Capability 검증
          </button>
          <button
            className={primaryButtonClass}
            disabled={busy || !validation?.valid}
            onClick={() =>
              void run('Task Assignment를 생성했습니다.', () =>
                providerPlatformApi.createAssignment(buildAssignment())
              )
            }
          >
            <CheckCircleIcon /> Assignment 생성
          </button>
        </div>
        {validation && (
          <div
            className={`rounded border p-base text-sm ${
              validation.valid
                ? 'border-success/50 bg-success/10 text-success'
                : 'border-error/50 bg-error/10 text-error'
            }`}
          >
            <div className="font-medium">
              {validation.valid
                ? '필수 Capability와 권한을 모두 충족합니다.'
                : '이 Assignment는 Task 요구사항을 충족하지 않습니다.'}
            </div>
            {validation.missing_capabilities.length > 0 && (
              <div className="mt-2">
                부족한 Capability: {validation.missing_capabilities.join(', ')}
              </div>
            )}
            {validation.missing_permissions.length > 0 && (
              <div className="mt-2">
                승인되지 않은 권한: {validation.missing_permissions.join(', ')}
              </div>
            )}
            {validation.warnings.map((warning) => (
              <div key={warning} className="mt-1">
                {warning}
              </div>
            ))}
          </div>
        )}
      </Panel>

      <Panel
        title="Session 실행 기록"
        description="Assignment에서 공급자별 reference와 metadata를 범용 필드로 저장합니다."
      >
        <div className="space-y-half">
          {snapshot.assignments.length === 0 && (
            <div className="text-sm text-low">아직 Assignment가 없습니다.</div>
          )}
          {snapshot.assignments.map((assignment) => {
            const plugin = snapshot.plugins.find(
              (item) => item.id === assignment.provider_plugin_id
            );
            const agent = snapshot.agent_profiles.find(
              (item) => item.id === assignment.agent_profile_id
            );
            return (
              <div
                key={assignment.id}
                className="flex items-center justify-between rounded bg-secondary p-base"
              >
                <div>
                  <div className="text-base text-high">
                    {assignment.task_id}
                  </div>
                  <div className="text-xs text-low">
                    {plugin?.display_name ?? assignment.provider_plugin_id} ·{' '}
                    {agent?.name ?? assignment.agent_profile_id}
                  </div>
                </div>
                <button
                  className={buttonClass}
                  disabled={busy}
                  onClick={() =>
                    void run('Provider Session 실행을 요청했습니다.', () =>
                      providerPlatformApi.startSession({
                        assignment_id: assignment.id,
                        context_package_id: null,
                        mode: 'interactive',
                        input: { taskId: assignment.task_id },
                      })
                    )
                  }
                >
                  <PlayIcon /> Session 시작
                </button>
              </div>
            );
          })}
        </div>
      </Panel>
    </div>
  );
}

export { ProviderPlatformSettingsSection as ProviderPlatformSettingsSectionContent };
