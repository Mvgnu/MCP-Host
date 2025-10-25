CREATE TABLE organizations (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    owner_id INTEGER REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE organization_members (
    organization_id INTEGER REFERENCES organizations(id) ON DELETE CASCADE,
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    PRIMARY KEY (organization_id, user_id)
);

ALTER TABLE mcp_servers ADD COLUMN organization_id INTEGER REFERENCES organizations(id);
