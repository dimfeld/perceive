CREATE TABLE tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  color TEXT NOT NULL
);

CREATE INDEX tags_name_idx ON tags(name);

CREATE TABLE item_tags (
  item_id BIGINT NOT NULL REFERENCES items(id) ON DELETE CASCADE DEFERRABLE,
  tag_id BIGINT NOT NULL REFERENCES tags(id) ON DELETE CASCADE DEFERRABLE,
  PRIMARY KEY (item_id, tag_id)
);

CREATE INDEX item_tags_item_id_idx ON item_tags(item_id);
CREATE INDEX item_tags_tag_id_idx ON item_tags(tag_id);
