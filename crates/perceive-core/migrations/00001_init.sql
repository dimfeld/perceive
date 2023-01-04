CREATE TABLE config (
  key TEXT PRIMARY KEY,
  value TEXT
);

CREATE TABLE models (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  model_type TEXT NOT NULL,
  created_at BIGINT NOT NULL
);

CREATE TABLE model_versions (
  model_id INT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
  version INT NOT NULL DEFAULT 0,
  status TEXT NOT NULL,
  weights_filename TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  PRIMARY KEY(model_id, version)
);

CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  -- Configuration specific to the source
  config TEXT,
  location TEXT NOT NULL,
  -- ItemCompareStrategy
  compare_strategy TEXT NOT NULL,
  -- SourceStatus
  status TEXT NOT NULL,
  last_indexed BIGINT NOT NULL DEFAULT 0,
  -- The version of the index, updated when starting.
  index_version BIGINT NOT NULL DEFAULT 0,
  -- How often to reindex the source, in seconds.
  -- If NULL it will only be reindex manually.
  index_interval BIGINT
);

CREATE TABLE items (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
  -- The path inside the source for files, a URL for web pages, etc.
  external_id TEXT NOT NULL,
  version INTEGER NOT NULL DEFAULT 0,
  hash TEXT NOT NULL,
  content TEXT NOT NULL,
  -- For content that has been processed after reading, the original content
  raw_content BLOB,
  process_version INTEGER NOT NULL DEFAULT 0,
  -- Metadata that we may or may not be able to glean from the file
  name TEXT,
  author TEXT,
  description TEXT,
  modified BIGINT,
  last_accessed BIGINT,
  skipped TEXT,
  -- Set if the user chose to hide this item from the search results
  hidden_at BIGINT
);

CREATE INDEX items_source_external_id_idx ON items(source_id, external_id);

CREATE TABLE item_embeddings (
  model_id INT NOT NULL,
  model_version INT NOT NULL,
  item_id BIGINT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  item_index_version BIGINT NOT NULL,
  embedding BLOB NOT NULL,
  FOREIGN KEY(model_id, model_version) REFERENCES model_versions(model_id, version) ON DELETE CASCADE,
  PRIMARY KEY(model_id, model_version, item_id)
);

INSERT INTO models (id, name, model_type, created_at) VALUES
  (0, 'AllMiniLmL12V2', 'AllMiniLmL12V2', 0),
  (1, 'AllMiniLmL6V2', 'AllMiniLmL6V2', 0),
  (2, 'DistiluseBaseMultilingualCased', 'DistiluseBaseMultilingualCased', 0),
  (3, 'AllDistilrobertaV1', 'AllDistilrobertaV1', 0),
  (4, 'ParaphraseAlbertSmallV2', 'ParaphraseAlbertSmallV2', 0),
  (5, 'MsMarcoDistilbertBaseV4', 'MsMarcoDistilbertBaseV4', 0),
  (6, 'MsMarcoDistilbertBaseTasB', 'MsMarcoDistilbertBaseTasB', 0);

INSERT INTO model_versions (model_id, version, status, weights_filename, created_at) VALUES
  (0, 0, 'ready', '', 0),
  (1, 0, 'ready', '', 0),
  (2, 0, 'ready', '', 0),
  (3, 0, 'ready', '', 0),
  (4, 0, 'ready', '', 0),
  (5, 0, 'ready', '', 0),
  (6, 0, 'ready', '', 0);
