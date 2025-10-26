ALTER TABLE runtime_policy_decisions
    ADD COLUMN candidate_backend TEXT DEFAULT 'unknown',
    ADD COLUMN capability_requirements JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN capabilities_satisfied BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN executor_name TEXT;

UPDATE runtime_policy_decisions
SET candidate_backend = backend
WHERE candidate_backend IS NULL OR candidate_backend = 'unknown';

ALTER TABLE runtime_policy_decisions
    ALTER COLUMN candidate_backend SET NOT NULL;
