UPDATE mobility_snapshots
SET payload = jsonb_set(
    payload,
    '{last_processed_month}',
    to_jsonb((tick / 13140)::bigint),
    true
)
WHERE NOT (payload ? 'last_processed_month');
