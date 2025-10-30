// key: organizations-lib -> self-service-onboarding

export interface OrgInfo {
  id: number;
  name: string;
}

export interface Invitation {
  id: string;
  organization_id: number;
  email: string;
  status: string;
  invited_at: string;
  accepted_at?: string | null;
  expires_at: string;
  token: string;
}

async function handleJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || response.statusText);
  }
  if (response.status === 204) {
    return null as T;
  }
  return (await response.json()) as T;
}

export async function createOrganization(name: string): Promise<OrgInfo> {
  const response = await fetch('/api/orgs', {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name }),
  });
  return handleJson(response);
}

export async function listOrganizations(): Promise<OrgInfo[]> {
  const response = await fetch('/api/orgs', {
    credentials: 'include',
  });
  return handleJson(response);
}

export async function listInvitations(organizationId: number): Promise<Invitation[]> {
  const response = await fetch(`/api/orgs/${organizationId}/invitations`, {
    credentials: 'include',
  });
  return handleJson(response);
}

export async function createInvitation(
  organizationId: number,
  email: string,
  expiresAt?: string,
): Promise<Invitation> {
  const response = await fetch(`/api/orgs/${organizationId}/invitations`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, expires_at: expiresAt }),
  });
  return handleJson(response);
}

export async function acceptInvitation(token: string): Promise<Invitation> {
  const response = await fetch(`/api/orgs/invitations/${token}/accept`, {
    method: 'POST',
    credentials: 'include',
  });
  return handleJson(response);
}
