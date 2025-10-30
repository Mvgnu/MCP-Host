import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import VectorDbAttachmentList from '../VectorDbAttachmentList';
import type { VectorDbAttachmentRecord } from '../../../lib/vectorDbs';

describe('VectorDbAttachmentList', () => {
  const attachments: VectorDbAttachmentRecord[] = [
    {
      id: 'attach-1',
      vector_db_id: 44,
      attachment_type: 'semantic',
      attachment_ref: 'svc-1',
      residency_policy_id: 10,
      provider_key_binding_id: 'binding-1',
      provider_key_id: 'key-1',
      provider_key_rotation_due_at: '2025-12-25T00:00:00.000Z',
      attached_at: '2025-12-20T09:00:00.000Z',
      detached_at: null,
      detached_reason: null,
      metadata: { environment: 'prod' },
    },
  ];

  it('renders attachment metadata with rotation badge', () => {
    render(
      <VectorDbAttachmentList attachments={attachments} onDetach={jest.fn()} />,
    );

    expect(screen.getByText(/semantic/)).toBeInTheDocument();
    expect(screen.getByText(/binding-1/)).toBeInTheDocument();
    expect(screen.getByText(/Rotate by/)).toBeInTheDocument();
  });

  it('emits detach action with provided reason', async () => {
    const onDetach = jest.fn().mockResolvedValue(undefined);

    render(
      <VectorDbAttachmentList attachments={attachments} onDetach={onDetach} />,
    );

    fireEvent.change(screen.getByPlaceholderText(/Rotation or remediation note/), {
      target: { value: 'credential rotated' },
    });
    fireEvent.click(screen.getByText(/Detach attachment/));

    await waitFor(() => {
      expect(onDetach).toHaveBeenCalledWith('attach-1', { reason: 'credential rotated' });
    });
  });
});
