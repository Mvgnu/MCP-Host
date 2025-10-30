-- key: migration-self-service -> organization-invitations

BEGIN;

CREATE TABLE IF NOT EXISTS organization_invitations (
    id UUID PRIMARY KEY,
    organization_id INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    invited_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    email TEXT NOT NULL,
    token UUID NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending',
    invited_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '14 days'
);

CREATE INDEX IF NOT EXISTS idx_org_invitations_org ON organization_invitations(organization_id);
CREATE INDEX IF NOT EXISTS idx_org_invitations_status ON organization_invitations(status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_org_invitations_unique_pending
    ON organization_invitations(organization_id, email)
    WHERE status = 'pending';

COMMIT;

-- Down

BEGIN;

DROP TABLE IF EXISTS organization_invitations;

COMMIT;
