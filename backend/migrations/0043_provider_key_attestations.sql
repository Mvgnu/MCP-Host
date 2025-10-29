-- BYOK attestation hardening for signed provenance and operator capture
-- key: migration-provider-key-attestations

BEGIN;

ALTER TABLE provider_keys
    ADD COLUMN IF NOT EXISTS attestation_signature TEXT,
    ADD COLUMN IF NOT EXISTS attestation_verified_at TIMESTAMPTZ;

ALTER TABLE provider_key_rotations
    ADD COLUMN IF NOT EXISTS attestation_digest TEXT,
    ADD COLUMN IF NOT EXISTS attestation_signature TEXT;

UPDATE provider_key_rotations
SET request_actor_ref = COALESCE(request_actor_ref, 'unspecified');

ALTER TABLE provider_key_rotations
    ALTER COLUMN request_actor_ref SET NOT NULL;

COMMIT;

-- Down

BEGIN;

ALTER TABLE provider_key_rotations
    ALTER COLUMN request_actor_ref DROP NOT NULL;

ALTER TABLE provider_key_rotations
    DROP COLUMN IF EXISTS attestation_signature,
    DROP COLUMN IF EXISTS attestation_digest;

ALTER TABLE provider_keys
    DROP COLUMN IF EXISTS attestation_verified_at,
    DROP COLUMN IF EXISTS attestation_signature;

COMMIT;
