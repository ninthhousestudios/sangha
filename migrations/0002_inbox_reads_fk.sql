-- Add FK ON DELETE CASCADE so prune_old_inbox cleans up read-receipts,
-- and add an index on message_id for the LEFT JOIN in read_inbox.
--
-- SQLite cannot ALTER a table to add FKs, so we recreate inbox_reads.
-- Read-receipts are ephemeral — safe to drop and recreate.

DROP TABLE IF EXISTS inbox_reads;

CREATE TABLE inbox_reads (
    session_id TEXT NOT NULL,
    message_id INTEGER NOT NULL REFERENCES inbox(id) ON DELETE CASCADE,
    read_at INTEGER NOT NULL,
    PRIMARY KEY (session_id, message_id)
);

CREATE INDEX idx_inbox_reads_message ON inbox_reads(message_id);
