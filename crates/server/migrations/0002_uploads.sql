CREATE TABLE uploads (
    id TEXT PRIMARY KEY,
    user_sub TEXT NOT NULL,
    volume TEXT NOT NULL,
    path TEXT NOT NULL,
    length INTEGER NOT NULL,
    modified_ms INTEGER,
    conflict TEXT NOT NULL,
    final_path TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX uploads_user ON uploads (user_sub);
CREATE INDEX uploads_updated ON uploads (updated_at);
