-- Startup renders exact mailbox totals and unread/starred badges. Cover those
-- scans with compact indexes so SQLite does not read the much wider message
-- and thread table rows for every item in a large mailbox.
CREATE INDEX IF NOT EXISTS idx_messages_thread_folder_account
    ON messages(thread_id, folder_id, account_id);

-- Crash recovery resets any body fetch that was interrupted. Without this
-- partial index the no-op case still scans every wide message row on every
-- launch, even when no message is in the fetching state.
CREATE INDEX IF NOT EXISTS idx_messages_body_fetching
    ON messages(id)
    WHERE body_state = 'fetching';

DROP INDEX IF EXISTS idx_threads_unread;
CREATE INDEX idx_threads_unread
    ON threads(account_id, starred_count)
    WHERE unread_count > 0;
