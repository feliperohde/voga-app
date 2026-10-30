-- Candidacies now keep their coalition and federation (who a politician's allies are): the TSE
-- records already linked are read again once.
UPDATE sources SET status = 'active', next_run_at = '2000-01-01T00:00:00Z' WHERE kind = 'tse_candidato' AND status = 'done';
