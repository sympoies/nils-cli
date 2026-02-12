create table if not exists workflow_item_anchors (
  anchor_id integer primary key,
  item_id integer not null references inbox_items(item_id) on delete cascade,
  workflow_type text not null,
  created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  check(length(trim(workflow_type)) > 0),
  unique(item_id, workflow_type)
);

create index if not exists idx_workflow_item_anchors_type_item
  on workflow_item_anchors(workflow_type, item_id);

create table if not exists workflow_game_entries (
  game_entry_id integer primary key,
  anchor_id integer not null references workflow_item_anchors(anchor_id) on delete cascade,
  game_name text not null,
  source_url text,
  description text,
  created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  check(length(trim(game_name)) > 0)
);

create index if not exists idx_workflow_game_entries_anchor
  on workflow_game_entries(anchor_id);
