begin;

create table if not exists cards (
  id text primary key,
  name text not null,
  type text not null check (type in ('attack', 'skill', 'power', 'status', 'curse')),
  mana_cost int not null check (mana_cost >= -2),
  description text,
  rarity text not null check (rarity in ('starter', 'common', 'uncommon', 'rare', 'special', 'curse')),
  version int not null default 1,
  updated_at timestamptz not null default now()
);

alter table cards enable row level security;

drop policy if exists cards_read on cards;
create policy cards_read on cards
  for select to authenticated
  using (true);

insert into cards (id, name, type, mana_cost, description, rarity) values
  ('strike', 'Strike', 'attack', 1, 'Deal damage.', 'starter'),
  ('defend', 'Defend', 'skill', 1, 'Gain block.', 'starter'),
  ('bash', 'Bash', 'attack', 2, 'A heavier starter attack.', 'starter'),
  ('guard', 'Guard', 'skill', 1, 'Prepare a stable defense.', 'common'),
  ('focus', 'Focus', 'power', 1, 'Keep this card as reusable content.', 'common')
on conflict (id) do update set
  name = excluded.name,
  type = excluded.type,
  mana_cost = excluded.mana_cost,
  description = excluded.description,
  rarity = excluded.rarity,
  version = cards.version + 1,
  updated_at = now();

create table if not exists user_card_hands (
  user_id uuid primary key references auth.users(id) on delete cascade,
  cards jsonb not null,
  updated_at timestamptz not null default now(),
  constraint user_card_hands_cards_array check (jsonb_typeof(cards) = 'array')
);

alter table user_card_hands enable row level security;

drop policy if exists user_card_hands_owner on user_card_hands;
create policy user_card_hands_owner on user_card_hands
  for all to authenticated
  using (user_id = auth.uid())
  with check (user_id = auth.uid());

commit;
