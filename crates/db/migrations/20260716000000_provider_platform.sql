CREATE TABLE provider_plugins (
    id                    TEXT PRIMARY KEY NOT NULL,
    plugin_key            TEXT NOT NULL UNIQUE,
    display_name          TEXT NOT NULL,
    version               TEXT NOT NULL,
    vendor                TEXT,
    description           TEXT,
    manifest_path         TEXT NOT NULL,
    adapter_entrypoint    TEXT NOT NULL,
    configuration_schema  TEXT NOT NULL DEFAULT '{}',
    credential_schema     TEXT NOT NULL DEFAULT '{}',
    capabilities          TEXT NOT NULL DEFAULT '[]',
    runtime_types         TEXT NOT NULL DEFAULT '[]',
    permissions           TEXT NOT NULL DEFAULT '[]',
    status                TEXT NOT NULL DEFAULT 'installed',
    enabled               INTEGER NOT NULL DEFAULT 1,
    installed_at          TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at            TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE TABLE provider_accounts (
    id                    TEXT PRIMARY KEY NOT NULL,
    provider_plugin_id    TEXT NOT NULL REFERENCES provider_plugins(id) ON DELETE CASCADE,
    display_name          TEXT NOT NULL,
    account_type          TEXT NOT NULL,
    credential_reference  TEXT,
    configuration_json    TEXT NOT NULL DEFAULT '{}',
    status                TEXT NOT NULL DEFAULT 'unvalidated',
    enabled               INTEGER NOT NULL DEFAULT 1,
    last_validated_at     TEXT,
    created_at            TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at            TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE INDEX idx_provider_accounts_plugin
    ON provider_accounts(provider_plugin_id);

CREATE TABLE model_definitions (
    id                       TEXT PRIMARY KEY NOT NULL,
    provider_account_id      TEXT NOT NULL REFERENCES provider_accounts(id) ON DELETE CASCADE,
    provider_model_key       TEXT NOT NULL,
    display_name             TEXT NOT NULL,
    description              TEXT,
    context_window           INTEGER,
    input_modalities         TEXT NOT NULL DEFAULT '["text"]',
    output_modalities        TEXT NOT NULL DEFAULT '["text"]',
    capability_json          TEXT NOT NULL DEFAULT '[]',
    pricing_json             TEXT NOT NULL DEFAULT '{}',
    availability_status      TEXT NOT NULL DEFAULT 'available',
    discovered_automatically INTEGER NOT NULL DEFAULT 0,
    enabled                  INTEGER NOT NULL DEFAULT 1,
    created_at               TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at               TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    UNIQUE(provider_account_id, provider_model_key)
);

CREATE INDEX idx_model_definitions_account
    ON model_definitions(provider_account_id);

CREATE TABLE runtime_profiles (
    id                       TEXT PRIMARY KEY NOT NULL,
    provider_account_id      TEXT NOT NULL REFERENCES provider_accounts(id) ON DELETE CASCADE,
    name                     TEXT NOT NULL,
    runtime_type             TEXT NOT NULL,
    executable_path          TEXT,
    endpoint                 TEXT,
    remote_connection_id     TEXT,
    working_directory_policy TEXT NOT NULL DEFAULT 'workspace',
    environment_reference    TEXT,
    configuration_json       TEXT NOT NULL DEFAULT '{}',
    enabled                  INTEGER NOT NULL DEFAULT 1,
    created_at               TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at               TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE INDEX idx_runtime_profiles_account
    ON runtime_profiles(provider_account_id);

CREATE TABLE permission_policies (
    id                   TEXT PRIMARY KEY NOT NULL,
    name                 TEXT NOT NULL UNIQUE,
    description          TEXT,
    approved_permissions TEXT NOT NULL DEFAULT '[]',
    constraints_json     TEXT NOT NULL DEFAULT '{}',
    created_at           TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE TABLE agent_profiles (
    id                           TEXT PRIMARY KEY NOT NULL,
    name                         TEXT NOT NULL,
    description                  TEXT,
    role_key                     TEXT NOT NULL,
    system_instructions          TEXT NOT NULL DEFAULT '',
    compatible_capabilities      TEXT NOT NULL DEFAULT '[]',
    preferred_provider_plugin_id TEXT REFERENCES provider_plugins(id) ON DELETE SET NULL,
    preferred_model_id           TEXT REFERENCES model_definitions(id) ON DELETE SET NULL,
    allowed_tools                TEXT NOT NULL DEFAULT '[]',
    denied_tools                 TEXT NOT NULL DEFAULT '[]',
    permission_policy_id         TEXT REFERENCES permission_policies(id) ON DELETE SET NULL,
    context_policy_json          TEXT NOT NULL DEFAULT '{}',
    enabled                      INTEGER NOT NULL DEFAULT 1,
    created_at                   TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at                   TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE TABLE assignments (
    id                         TEXT PRIMARY KEY NOT NULL,
    task_id                    TEXT NOT NULL,
    provider_plugin_id         TEXT NOT NULL REFERENCES provider_plugins(id),
    provider_account_id        TEXT NOT NULL REFERENCES provider_accounts(id),
    model_definition_id        TEXT REFERENCES model_definitions(id),
    runtime_profile_id         TEXT NOT NULL REFERENCES runtime_profiles(id),
    agent_profile_id           TEXT NOT NULL REFERENCES agent_profiles(id),
    permission_policy_id       TEXT REFERENCES permission_policies(id),
    required_capabilities      TEXT NOT NULL DEFAULT '[]',
    status                     TEXT NOT NULL DEFAULT 'assigned',
    assigned_by                TEXT NOT NULL DEFAULT 'user',
    assigned_at                TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    started_at                 TEXT,
    ended_at                   TEXT,
    handoff_from_assignment_id TEXT REFERENCES assignments(id),
    handoff_reason             TEXT
);

CREATE INDEX idx_assignments_task ON assignments(task_id);

CREATE TABLE provider_sessions (
    id                         TEXT PRIMARY KEY NOT NULL,
    assignment_id              TEXT NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    provider_session_reference TEXT,
    provider_thread_reference  TEXT,
    external_session_reference TEXT,
    resume_strategy            TEXT NOT NULL DEFAULT 'new_session',
    mode                       TEXT NOT NULL DEFAULT 'interactive',
    status                     TEXT NOT NULL DEFAULT 'created',
    context_package_id         TEXT,
    started_at                 TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    ended_at                   TEXT,
    last_message_at            TEXT,
    summary                    TEXT,
    provider_metadata_json     TEXT NOT NULL DEFAULT '{}',
    error_code                 TEXT,
    error_message              TEXT
);

CREATE INDEX idx_provider_sessions_assignment
    ON provider_sessions(assignment_id);

CREATE TABLE provider_session_events (
    id          TEXT PRIMARY KEY NOT NULL,
    session_id  TEXT NOT NULL REFERENCES provider_sessions(id) ON DELETE CASCADE,
    event_type  TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE INDEX idx_provider_session_events_session
    ON provider_session_events(session_id, created_at);

