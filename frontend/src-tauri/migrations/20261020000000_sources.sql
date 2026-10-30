-- More sources: each target also searches the municipal gazettes (Querido Diário).
INSERT OR IGNORE INTO sources (id, investigation_id, target_id, kind, url, title, every_minutes, next_run_at, origin, created_at)
SELECT 'src-' || lower(hex(randomblob(16))), t.investigation_id, t.id, k.kind, '"' || t.name || '"', t.name, 1440,
       strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'target', strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM targets t CROSS JOIN (SELECT 'gazette_search' AS kind) k;
