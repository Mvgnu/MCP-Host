// key: byok-console-contract
// Provider BYOK client placeholders used by the lifecycle console and provider portal.

export type ProviderKeyState =
  | 'pending_registration'
  | 'active'
  | 'rotating'
  | 'retired'
  | 'compromised';

export interface ProviderKeyRecord {
  id: string;
  provider_id: string;
  alias?: string | null;
  state: ProviderKeyState;
  rotation_due_at?: string | null;
  attestation_digest?: string | null;
  attestation_signature_registered?: boolean;
  attestation_verified_at?: string | null;
  activated_at?: string | null;
  retired_at?: string | null;
  compromised_at?: string | null;
  version: number;
  created_at: string;
  updated_at: string;
}

export interface ProviderKeyDecisionPosture {
  provider_id: string;
  provider_key_id?: string | null;
  tier?: string | null;
  state?: ProviderKeyState | null;
  rotation_due_at?: string | null;
  attestation_registered: boolean;
  attestation_signature_verified: boolean;
  attestation_verified_at?: string | null;
  vetoed: boolean;
  notes?: string[];
}

export interface ProviderKeyClient {
  listKeys(providerId: string): Promise<ProviderKeyRecord[]>;
  registerKey(
    providerId: string,
    payload: {
      alias?: string | null;
      attestationDigest?: string | null;
      attestationSignature?: string | null;
      rotationDueAt?: string | null;
    }
  ): Promise<ProviderKeyRecord>;
}

const NOT_IMPLEMENTED =
  'Provider BYOK APIs are not yet implemented. Upgrade once the backend exposes the contract.';

export async function fetchProviderKeys(
  providerId: string
): Promise<ProviderKeyRecord[]> {
  void providerId;
  throw new Error(NOT_IMPLEMENTED);
}

export async function registerProviderKey(
  providerId: string,
  alias?: string | null
): Promise<ProviderKeyRecord> {
  void providerId;
  void alias;
  throw new Error(NOT_IMPLEMENTED);
}

export function isRotationOverdue(record: ProviderKeyRecord): boolean {
  if (!record.rotation_due_at) {
    return false;
  }
  const deadline = new Date(record.rotation_due_at).getTime();
  return Number.isFinite(deadline) && deadline < Date.now();
}
