-- Every feature is always available: no trial and no keys any more.
DROP TABLE IF EXISTS entitlement_keys;
DROP TABLE IF EXISTS entitlement_state;

-- Sources that stopped when the trial ended kept {"locked": "<feature>"} as their error.
UPDATE sources SET last_error = NULL WHERE last_error LIKE '%{"locked"%';
