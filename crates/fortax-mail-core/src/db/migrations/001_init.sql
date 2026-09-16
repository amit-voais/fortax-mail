CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  display_name TEXT,
  provider TEXT NOT NULL CHECK (provider IN ('imap','gmail','microsoft')),
  auth_kind TEXT NOT NULL CHECK (auth_kind IN ('password','oauth2')),
  mail_protocol TEXT NOT NULL DEFAULT 'imap'
    CHECK (mail_protocol IN ('imap', 'jmap')),
  username TEXT NOT NULL,
  jmap_url TEXT NOT NULL DEFAULT '',
  jmap_account_id TEXT,
  imap_host TEXT NOT NULL,
  imap_port INTEGER NOT NULL,
  smtp_host TEXT NOT NULL,
  smtp_port INTEGER NOT NULL,
  sync_state TEXT NOT NULL DEFAULT 'idle',
  sync_error TEXT,
  created_at INTEGER NOT NULL,
  settings_json TEXT NOT NULL DEFAULT '{}',
  sort_order INTEGER NOT NULL DEFAULT 0,
  avatar_url TEXT
);
CREATE INDEX idx_accounts_sort_order ON accounts(sort_order, id);

CREATE TABLE folders (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  imap_name TEXT NOT NULL,
  delimiter TEXT,
  role TEXT,
  uidvalidity INTEGER,
  uidnext INTEGER,
  highestmodseq INTEGER,
  last_seen_uid INTEGER NOT NULL DEFAULT 0,
  backfill_cursor INTEGER,
  backfill_done INTEGER NOT NULL DEFAULT 0,
  jmap_id TEXT,
  UNIQUE(account_id, imap_name)
);
CREATE UNIQUE INDEX idx_folders_jmap_id
  ON folders(account_id, jmap_id) WHERE jmap_id IS NOT NULL;

CREATE TABLE threads (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  gm_thrid TEXT,
  jmap_id TEXT,
  subject_norm TEXT,
  last_message_at INTEGER NOT NULL DEFAULT 0,
  message_count INTEGER NOT NULL DEFAULT 0,
  unread_count INTEGER NOT NULL DEFAULT 0,
  starred_count INTEGER NOT NULL DEFAULT 0,
  attachment_count INTEGER NOT NULL DEFAULT 0,
  snippet TEXT NOT NULL DEFAULT '',
  participants_json TEXT NOT NULL DEFAULT '[]',
  routed_tab TEXT
);
CREATE INDEX idx_threads_recent
  ON threads(account_id, last_message_at DESC, id DESC);
CREATE INDEX idx_threads_gm ON threads(account_id, gm_thrid);
CREATE INDEX idx_threads_subj ON threads(account_id, subject_norm);
CREATE INDEX idx_threads_last_msg ON threads(last_message_at DESC, id DESC);
CREATE INDEX idx_threads_unread ON threads(account_id) WHERE unread_count > 0;
CREATE INDEX idx_threads_routed_tab ON threads(routed_tab);
CREATE UNIQUE INDEX idx_threads_jmap_id
  ON threads(account_id, jmap_id) WHERE jmap_id IS NOT NULL;

CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  thread_id INTEGER REFERENCES threads(id),
  folder_id INTEGER REFERENCES folders(id),
  uid INTEGER,
  message_id TEXT,
  gm_msgid TEXT,
  gm_thrid TEXT,
  subject TEXT NOT NULL DEFAULT '',
  from_name TEXT,
  from_addr TEXT,
  sender_addr TEXT,
  to_json TEXT NOT NULL DEFAULT '[]',
  cc_json TEXT NOT NULL DEFAULT '[]',
  bcc_json TEXT NOT NULL DEFAULT '[]',
  date INTEGER NOT NULL,
  internal_date INTEGER,
  is_read INTEGER NOT NULL DEFAULT 0,
  is_starred INTEGER NOT NULL DEFAULT 0,
  is_draft INTEGER NOT NULL DEFAULT 0,
  is_outgoing INTEGER NOT NULL DEFAULT 0,
  is_automated INTEGER NOT NULL DEFAULT 0,
  has_attachments INTEGER NOT NULL DEFAULT 0,
  size INTEGER,
  snippet TEXT NOT NULL DEFAULT '',
  body_state TEXT NOT NULL DEFAULT 'none' CHECK (body_state IN ('none','fetching','cached')),
  raw_path TEXT,
  list_unsubscribe TEXT,
  list_unsubscribe_post TEXT,
  embedding_state TEXT NOT NULL DEFAULT 'none' CHECK (embedding_state IN ('none','pending','done')),
  mime_plan_json TEXT,
  local_subject_prefix TEXT NOT NULL DEFAULT '',
  local_body_note TEXT NOT NULL DEFAULT '',
  sender_verification TEXT NOT NULL DEFAULT ''
    CHECK (sender_verification IN ('', 'domain', 'microsoft', 'bimi', 'brand')),
  gmail_sync_generation INTEGER,
  gmail_draft_id TEXT,
  jmap_id TEXT,
  jmap_blob_id TEXT,
  UNIQUE(account_id, folder_id, uid)
);
CREATE INDEX idx_messages_thread ON messages(thread_id, date);
CREATE INDEX idx_messages_msgid ON messages(account_id, message_id);
CREATE INDEX idx_messages_folder_uid ON messages(folder_id, uid);
CREATE INDEX idx_messages_gm_msgid ON messages(account_id, gm_msgid);
CREATE INDEX idx_messages_embedding_pending ON messages(date DESC)
  WHERE embedding_state = 'pending';
CREATE INDEX idx_messages_gmail_generation
  ON messages(account_id, gmail_sync_generation) WHERE gm_msgid IS NOT NULL;
CREATE INDEX idx_messages_gmail_draft
  ON messages(account_id, gmail_draft_id) WHERE gmail_draft_id IS NOT NULL;
CREATE UNIQUE INDEX idx_messages_jmap_id
  ON messages(account_id, jmap_id) WHERE jmap_id IS NOT NULL;

CREATE TABLE message_bodies (
  message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  text_body TEXT,
  html_body TEXT
);

CREATE TABLE message_refs (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  ref_message_id TEXT NOT NULL,
  PRIMARY KEY (message_id, ref_message_id)
) WITHOUT ROWID;
CREATE INDEX idx_refs_lookup ON message_refs(ref_message_id);

CREATE TABLE attachments (
  id INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  part_id TEXT,
  filename TEXT,
  mime_type TEXT,
  size INTEGER,
  content_id TEXT,
  is_inline INTEGER NOT NULL DEFAULT 0,
  file_path TEXT,
  imap_section TEXT,
  provider_attachment_id TEXT
);
CREATE INDEX idx_attachments_msg ON attachments(message_id);
CREATE UNIQUE INDEX idx_attachments_imap_section
  ON attachments(message_id, imap_section) WHERE imap_section IS NOT NULL;
CREATE UNIQUE INDEX idx_attachments_provider_id
  ON attachments(message_id, provider_attachment_id)
  WHERE provider_attachment_id IS NOT NULL;

CREATE TABLE contacts (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name TEXT,
  send_count INTEGER NOT NULL DEFAULT 0,
  recv_count INTEGER NOT NULL DEFAULT 0,
  last_interacted INTEGER,
  folded TEXT,
  phone TEXT NOT NULL DEFAULT '',
  company TEXT NOT NULL DEFAULT '',
  job_title TEXT NOT NULL DEFAULT '',
  website TEXT NOT NULL DEFAULT '',
  birthday TEXT NOT NULL DEFAULT '',
  postal_address TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '',
  tags TEXT NOT NULL DEFAULT '',
  is_favorite INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
  is_managed INTEGER NOT NULL DEFAULT 0 CHECK (is_managed IN (0, 1)),
  updated_at INTEGER
);
CREATE INDEX idx_contacts_rank ON contacts(send_count DESC, last_interacted DESC);
CREATE INDEX idx_contacts_directory
  ON contacts(is_favorite DESC, name COLLATE NOCASE, email COLLATE NOCASE);

CREATE TABLE contact_accounts (
  contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  send_count INTEGER NOT NULL DEFAULT 0,
  recv_count INTEGER NOT NULL DEFAULT 0,
  last_interacted INTEGER,
  PRIMARY KEY (contact_id, account_id)
) WITHOUT ROWID;
CREATE INDEX idx_contact_accounts_acct ON contact_accounts(account_id);

CREATE TABLE pending_actions (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
  thread_id INTEGER,
  payload TEXT NOT NULL DEFAULT '{}',
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending','inflight','done','failed','cancelled')),
  attempts INTEGER NOT NULL DEFAULT 0,
  not_before INTEGER,
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  last_error TEXT
);
CREATE INDEX idx_actions_due
  ON pending_actions(account_id, state, not_before, created_at);

CREATE TABLE snoozes (
  thread_id INTEGER PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL,
  wake_at INTEGER NOT NULL,
  orig_folder_id INTEGER
);
CREATE INDEX idx_snoozes_wake ON snoozes(wake_at);

CREATE TABLE snippets (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  shortcut TEXT UNIQUE,
  subject TEXT,
  body_text TEXT NOT NULL DEFAULT '',
  usage_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE split_rules (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  query_json TEXT NOT NULL DEFAULT '{}',
  target TEXT
);

CREATE TABLE route_cache (
  sender_domain TEXT PRIMARY KEY,
  route_key TEXT NOT NULL
);

CREATE TABLE drafts_meta (
  message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  mode TEXT NOT NULL DEFAULT 'new',
  in_reply_to_message_id INTEGER,
  remote_uid INTEGER
);

CREATE TABLE draft_attachments (
  id INTEGER PRIMARY KEY,
  draft_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  filename TEXT NOT NULL,
  mime_type TEXT
);
CREATE INDEX idx_draft_attachments ON draft_attachments(draft_id);

CREATE TABLE app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE labels (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  color TEXT NOT NULL DEFAULT '#6b7280',
  keyword TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  is_auto INTEGER NOT NULL DEFAULT 0
);

INSERT INTO labels (name, color, keyword, position, is_auto) VALUES
  ('Marketing', '#e0708a', 'FortaxMailAutoMarketing', 1000, 1),
  ('News',      '#5b9dd9', 'FortaxMailAutoNews',      1001, 1),
  ('Social',    '#7bc47f', 'FortaxMailAutoSocial',    1002, 1),
  ('Pitch',     '#c9a04e', 'FortaxMailAutoPitch',     1003, 1);

CREATE TABLE message_labels (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY (message_id, label_id)
);
CREATE INDEX idx_message_labels_label ON message_labels(label_id, message_id);

CREATE TABLE message_embeddings (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  chunk_index INTEGER NOT NULL,
  model_id TEXT NOT NULL,
  dim INTEGER NOT NULL,
  vec BLOB NOT NULL,
  PRIMARY KEY (message_id, chunk_index, model_id)
);
CREATE INDEX idx_embeddings_model ON message_embeddings(model_id);

CREATE TABLE ai_usage_events (
  id INTEGER PRIMARY KEY,
  occurred_at INTEGER NOT NULL,
  model TEXT NOT NULL,
  scenario TEXT NOT NULL,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  total_tokens INTEGER NOT NULL DEFAULT 0,
  exact INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_ai_usage_occurred ON ai_usage_events(occurred_at);

CREATE TABLE sync_failures (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  stage TEXT NOT NULL CHECK (stage IN ('header','content')),
  folder_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
  message_id INTEGER REFERENCES messages(id) ON DELETE CASCADE,
  uid INTEGER,
  attempts INTEGER NOT NULL DEFAULT 1,
  next_retry_at INTEGER,
  last_error TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  CHECK (
    (stage = 'header' AND folder_id IS NOT NULL AND uid IS NOT NULL AND message_id IS NULL)
    OR
    (stage = 'content' AND folder_id IS NULL AND uid IS NULL AND message_id IS NOT NULL)
  )
);
CREATE UNIQUE INDEX idx_sync_failures_header
  ON sync_failures(folder_id, uid) WHERE stage = 'header';
CREATE UNIQUE INDEX idx_sync_failures_content
  ON sync_failures(message_id) WHERE stage = 'content';
CREATE INDEX idx_sync_failures_due
  ON sync_failures(account_id, stage, next_retry_at, updated_at);

CREATE TABLE notification_outbox (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
  thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
  sender_name TEXT,
  sender_addr TEXT,
  subject TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending','delivering','delivered','suppressed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  not_before INTEGER,
  created_at INTEGER NOT NULL,
  claimed_at INTEGER,
  delivered_at INTEGER,
  suppressed_at INTEGER,
  suppression_reason TEXT,
  last_error TEXT
);
CREATE INDEX idx_notification_outbox_due
  ON notification_outbox(state, not_before, created_at);

CREATE TABLE gmail_sync_state (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  sync_cursor TEXT,
  history_id TEXT,
  history_page_token TEXT,
  generation INTEGER NOT NULL DEFAULT 0,
  full_started_at INTEGER,
  backfill_done INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE jmap_sync_state (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  mailbox_state TEXT,
  email_state TEXT,
  identity_state TEXT,
  last_full_sync INTEGER
);

CREATE TABLE message_folders (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  PRIMARY KEY (message_id, folder_id)
);
CREATE INDEX idx_message_folders_folder ON message_folders(folder_id, message_id);

CREATE TABLE gmail_labels (
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  provider_id TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'system',
  folder_id INTEGER REFERENCES folders(id) ON DELETE SET NULL,
  local_label_id INTEGER REFERENCES labels(id) ON DELETE SET NULL,
  background_color TEXT,
  text_color TEXT,
  PRIMARY KEY (account_id, provider_id)
);
CREATE INDEX idx_gmail_labels_local ON gmail_labels(local_label_id, account_id);

CREATE TABLE cross_store_operations (
  kind TEXT NOT NULL,
  account_id INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (kind, account_id),
  CHECK (kind IN ('remove_account'))
);

CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, from_text, to_text, body,
  content='',
  contentless_delete=1,
  tokenize="unicode61 remove_diacritics 2"
);
