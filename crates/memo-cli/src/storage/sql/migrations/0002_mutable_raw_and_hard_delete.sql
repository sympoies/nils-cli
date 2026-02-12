drop trigger if exists trg_inbox_items_append_only_update;
drop trigger if exists trg_inbox_items_append_only_delete;

create trigger if not exists trg_inbox_items_au_refresh_search_document
after update of raw_text on inbox_items
begin
  update item_search_documents
  set raw_text = new.raw_text,
      updated_at = (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
  where item_id = new.item_id;
end;

create trigger if not exists trg_item_derivations_ad_refresh_search_document
after delete on item_derivations
begin
  update item_search_documents
  set derived_text = coalesce((
    select trim(
      coalesce(d.summary, '') || ' ' || coalesce(d.category, '') || ' ' || coalesce(d.normalized_text, '')
    )
    from item_derivations d
    where d.item_id = old.item_id
      and d.is_active = 1
      and d.status = 'accepted'
    order by d.derivation_version desc, d.derivation_id desc
    limit 1
  ), ''),
  tags_text = coalesce((
    select group_concat(t.tag_name, ' ')
    from item_tags it
    join tags t on t.tag_id = it.tag_id
    join item_derivations d on d.derivation_id = it.derivation_id
    where d.item_id = old.item_id
      and d.is_active = 1
      and d.status = 'accepted'
  ), ''),
  updated_at = (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
  where item_id = old.item_id;
end;
