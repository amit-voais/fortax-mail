-- Avoid scanning the entire directory when no legacy contact needs repair.
CREATE INDEX idx_contacts_unfolded ON contacts(id) WHERE folded IS NULL;
