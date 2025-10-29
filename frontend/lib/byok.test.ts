import { describe, expect, it } from '@jest/globals';

import { isRotationOverdue } from './byok';

describe('BYOK helpers', () => {
  it('detects overdue rotations using timestamps', () => {
    const record = {
      id: 'key-1',
      provider_id: 'tenant',
      state: 'active' as const,
      version: 1,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      rotation_due_at: new Date(Date.now() - 60_000).toISOString(),
    };

    expect(isRotationOverdue(record)).toBe(true);
  });

  it('treats missing deadlines as healthy', () => {
    const record = {
      id: 'key-2',
      provider_id: 'tenant',
      state: 'active' as const,
      version: 1,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };

    expect(isRotationOverdue(record)).toBe(false);
  });
});
