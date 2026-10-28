CREATE TABLE favorites (
    user_sub TEXT NOT NULL,
    volume TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_sub, volume, path)
);
