CREATE TABLE schema_info (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    created_at TEXT NOT NULL
);

INSERT INTO schema_info (id, created_at) VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
