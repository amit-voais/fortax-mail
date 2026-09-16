CREATE TABLE carddav_config (
  account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  base_url TEXT NOT NULL,
  username TEXT NOT NULL,
  principal_url TEXT,
  home_set_url TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  last_error TEXT
);

CREATE TABLE carddav_addressbooks (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  url TEXT NOT NULL,
  display_name TEXT,
  ctag TEXT,
  sync_token TEXT,
  read_only INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1)),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
  last_synced_at INTEGER,
  UNIQUE(account_id, url)
);
CREATE UNIQUE INDEX idx_carddav_one_default
  ON carddav_addressbooks(account_id) WHERE is_default = 1;

-- A contact can be present in several address books. The source object owns
-- the DAV identity and ETag while contacts keeps the app's unified-by-email
-- directory and mail affinity counters.
CREATE TABLE carddav_objects (
  id INTEGER PRIMARY KEY,
  addressbook_id INTEGER NOT NULL REFERENCES carddav_addressbooks(id) ON DELETE CASCADE,
  contact_id INTEGER REFERENCES contacts(id) ON DELETE SET NULL,
  href TEXT NOT NULL,
  etag TEXT,
  vcard_raw TEXT NOT NULL DEFAULT '',
  remote_exists INTEGER NOT NULL DEFAULT 1 CHECK (remote_exists IN (0, 1)),
  dirty INTEGER NOT NULL DEFAULT 0 CHECK (dirty IN (0, 1)),
  deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
  owns_contact INTEGER NOT NULL DEFAULT 0 CHECK (owns_contact IN (0, 1)),
  UNIQUE(addressbook_id, href)
);
CREATE INDEX idx_carddav_objects_contact ON carddav_objects(contact_id);
CREATE INDEX idx_carddav_objects_dirty ON carddav_objects(dirty, deleted)
  WHERE dirty = 1 OR deleted = 1;
