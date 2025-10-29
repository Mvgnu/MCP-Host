-- Persist promotion posture verdict metadata for downstream consumers
ALTER TABLE artifact_promotions
    ADD COLUMN IF NOT EXISTS posture_verdict JSONB;
