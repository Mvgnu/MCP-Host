-- Capture cadence metadata and governance annotations for evaluation evidence
ALTER TABLE evaluation_certifications
    ADD COLUMN refresh_cadence_seconds BIGINT,
    ADD COLUMN evidence_source JSONB,
    ADD COLUMN next_refresh_at TIMESTAMPTZ,
    ADD COLUMN governance_notes TEXT,
    ADD COLUMN evidence_lineage JSONB;

CREATE INDEX evaluation_certifications_next_refresh_idx
    ON evaluation_certifications (next_refresh_at)
    WHERE next_refresh_at IS NOT NULL;
