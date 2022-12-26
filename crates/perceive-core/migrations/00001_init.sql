CREATE TABLE models (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  model_type TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  deleted_at BIGINT
);

CREATE TABLE model_versions (
  model_id BIGINT NOT NULL REFERENCES models(id),
  version INTEGER NOR NULL DEFAULT 0,
  status TEXT NOT NULL,
  weights_filename TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  deleted_at BIGINT,
  PRIMARY KEY(model_id, version)
);

CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  source_type TEXT NOT NULL,
  location TEXT NOT NULL,
  last_indexed BIGINT NOT NULL DEFAULT 0,
  preferred_model BIGINT REFERENCES models(id),
  deleted_at BIGINT
);

CREATE TABLE items (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES sources(id),
  -- The path inside the source for files, a URL for web pages, etc.
  external_id TEXT NOT NULL,
  version TEXT NOT NULL DEFAULT 0,
  hash TEXT NOT NULL,
  content TEXT NOT NULL,
  -- Metadata that we may or may not be able to glean from the file
  name TEXT,
  author TEXT,
  description TEXT,
  modified BIGINT,
  last_accessed BIGINT,
  -- Set if the user chose to hide this item from the search results
  hidden_at BIGINT
);

CREATE INDEX ON items(source_id, external_id);

CREATE TABLE item_embeddings (
  model_id BIGINT NOT NULL,
  model_version BIGINT NOT NULL,
  item_id BIGINT NOT NULL REFERENCES items(id),
  embedding BLOB NOT NULL,
  FOREIGN KEY(model_id, model_version) REFERENCES model_versions(model_id, version),
  PRIMARY KEY(model_id, model_version, item_id)
);
