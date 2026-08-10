CREATE TABLE ai_room_local_identity (
    singleton   INTEGER PRIMARY KEY NOT NULL DEFAULT 1 CHECK (singleton = 1),
    owner_id    TEXT NOT NULL UNIQUE,
    device_id   TEXT NOT NULL UNIQUE,
    device_name TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE TABLE ai_room_storage_profiles (
    room_id     TEXT PRIMARY KEY NOT NULL REFERENCES ai_rooms(id) ON DELETE CASCADE,
    owner_id    TEXT NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'LOCAL_ONLY'
                CHECK (mode IN ('LOCAL_ONLY', 'TASK_AI_CLOUD', 'PERSONAL_HUB')),
    endpoint    TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    CHECK (mode != 'LOCAL_ONLY' OR endpoint IS NULL)
);

CREATE INDEX idx_ai_room_storage_profiles_owner
    ON ai_room_storage_profiles(owner_id);
