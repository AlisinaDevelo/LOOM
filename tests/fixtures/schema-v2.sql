CREATE TABLE schema_meta(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE source_roots(
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('file', 'directory')),
    locator TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL
) STRICT;

CREATE TABLE artifacts(
    id TEXT PRIMARY KEY,
    source_root_id TEXT NOT NULL REFERENCES source_roots(id),
    title TEXT NOT NULL,
    media_type TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('active', 'missing', 'tombstoned')),
    active_version_id TEXT REFERENCES artifact_versions(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL
) STRICT;

CREATE TABLE artifact_locators(
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('file', 'url', 'managed_copy')),
    locator TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    UNIQUE(kind, locator)
) STRICT;
CREATE UNIQUE INDEX artifact_one_active_locator
  ON artifact_locators(artifact_id) WHERE active = 1;

CREATE TABLE artifact_versions(
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    hash_algorithm TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
    source_modified_ns INTEGER,
    extractor_id TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('ready', 'failed', 'superseded')),
    created_at TEXT NOT NULL,
    UNIQUE(artifact_id, content_hash, extractor_id, extractor_version)
) STRICT;

CREATE TABLE passages(
    id TEXT PRIMARY KEY,
    artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    text TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    locator_json TEXT NOT NULL CHECK(json_valid(locator_json)),
    char_start INTEGER NOT NULL CHECK(char_start >= 0),
    char_end INTEGER NOT NULL CHECK(char_end >= char_start),
    line_start INTEGER NOT NULL CHECK(line_start >= 1),
    line_end INTEGER NOT NULL CHECK(line_end >= line_start),
    created_at TEXT NOT NULL,
    UNIQUE(artifact_version_id, ordinal)
) STRICT;

CREATE TABLE relationships(
    id TEXT PRIMARY KEY,
    source_artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    target_artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    evidence_passage_id TEXT REFERENCES passages(id) ON DELETE SET NULL,
    confidence REAL,
    method TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK(source_artifact_id <> target_artifact_id)
) STRICT;

CREATE VIRTUAL TABLE passages_fts USING fts5(
    text,
    content = 'passages',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER passages_ai AFTER INSERT ON passages BEGIN
    INSERT INTO passages_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER passages_ad AFTER DELETE ON passages BEGIN
    INSERT INTO passages_fts(passages_fts, rowid, text)
      VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER passages_au AFTER UPDATE ON passages BEGIN
    INSERT INTO passages_fts(passages_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
    INSERT INTO passages_fts(rowid, text) VALUES (new.rowid, new.text);
END;

INSERT INTO schema_meta(key, value) VALUES ('schema_version', '2');
INSERT INTO source_roots(id, kind, locator, enabled, created_at, last_seen_at)
VALUES ('root-v2', 'directory', 'fixture://loom-v2/root', 1,
        '2026-08-24T00:00:00Z', '2026-08-24T00:00:00Z');
INSERT INTO artifacts(
    id, source_root_id, title, media_type, state, active_version_id, created_at, last_seen_at
)
VALUES ('artifact-v2', 'root-v2', 'schema-v2.md', 'text/markdown', 'active', NULL,
        '2026-08-24T00:00:00Z', '2026-08-24T00:00:00Z');
INSERT INTO artifact_locators(
    id, artifact_id, kind, locator, active, first_seen_at, last_seen_at
)
VALUES ('locator-v2', 'artifact-v2', 'file', 'fixture://loom-v2/root/schema-v2.md', 1,
        '2026-08-24T00:00:00Z', '2026-08-24T00:00:00Z');
INSERT INTO artifact_versions(
    id, artifact_id, content_hash, hash_algorithm, byte_size, source_modified_ns,
    extractor_id, extractor_version, status, created_at
)
VALUES ('version-v2', 'artifact-v2', 'blake3:fixture-v2-hash', 'blake3', 42, 123,
        'loom.text', '0.1.0', 'ready', '2026-08-24T00:00:00Z');
UPDATE artifacts SET active_version_id = 'version-v2' WHERE id = 'artifact-v2';
INSERT INTO passages(
    id, artifact_version_id, ordinal, text, text_hash, locator_json,
    char_start, char_end, line_start, line_end, created_at
)
VALUES ('passage-v2', 'version-v2', 0, 'schema migration preserves anchors',
        'blake3:fixture-v2-passage',
        '{"kind":"text","char_start":0,"char_end":34,"line_start":1,"line_end":1}',
        0, 34, 1, 1, '2026-08-24T00:00:00Z');
INSERT INTO artifacts(
    id, source_root_id, title, media_type, state, active_version_id, created_at, last_seen_at
)
VALUES ('artifact-v2-target', 'root-v2', 'target.md', 'text/markdown', 'active', NULL,
        '2026-08-24T00:00:00Z', '2026-08-24T00:00:00Z');

INSERT INTO relationships(
    id, source_artifact_id, target_artifact_id, kind, evidence_passage_id,
    confidence, method, created_at
)
VALUES ('relationship-v2', 'artifact-v2', 'artifact-v2-target', 'related', 'passage-v2',
        0.75, 'fixture', '2026-08-24T00:00:00Z');

INSERT INTO passages_fts(rowid, text) SELECT rowid, text FROM passages;
