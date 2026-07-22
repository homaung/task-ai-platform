CREATE TABLE ai_rooms (
    id                  TEXT PRIMARY KEY NOT NULL,
    name                TEXT NOT NULL,
    description         TEXT,
    local_root          TEXT NOT NULL UNIQUE,
    ssh_alias           TEXT,
    remote_root         TEXT,
    instruction_version INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    CHECK (
        (ssh_alias IS NULL AND remote_root IS NULL) OR
        (ssh_alias IS NOT NULL AND remote_root IS NOT NULL)
    )
);

CREATE INDEX idx_ai_rooms_name ON ai_rooms(name);

