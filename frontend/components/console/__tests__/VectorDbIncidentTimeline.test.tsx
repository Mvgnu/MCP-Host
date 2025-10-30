import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import VectorDbIncidentTimeline from '../VectorDbIncidentTimeline';
import type { VectorDbIncidentRecord } from '../../../lib/vectorDbs';

describe('VectorDbIncidentTimeline', () => {
  const incidents: VectorDbIncidentRecord[] = [
    {
      id: 'incident-1',
      vector_db_id: 44,
      attachment_id: 'attach-1',
      incident_type: 'residency_breach',
      severity: 'high',
      occurred_at: '2025-12-20T10:00:00.000Z',
      resolved_at: null,
      summary: 'data replicated to eu-west',
      notes: { action: 'rollback' },
    },
  ];

  it('renders incident details and open status badge', () => {
    render(
      <VectorDbIncidentTimeline incidents={incidents} onResolve={jest.fn()} />,
    );

    expect(screen.getByText(/residency_breach/)).toBeInTheDocument();
    expect(screen.getByText(/Open/)).toBeInTheDocument();
    expect(screen.getByText(/rollback/)).toBeInTheDocument();
  });

  it('submits resolution payload', async () => {
    const onResolve = jest.fn().mockResolvedValue(undefined);

    render(
      <VectorDbIncidentTimeline incidents={incidents} onResolve={onResolve} />,
    );

    fireEvent.change(screen.getByLabelText('Resolution summary'), {
      target: { value: 'replica destroyed' },
    });
    fireEvent.change(screen.getByLabelText('Resolution notes'), {
      target: { value: 'verified by compliance' },
    });
    fireEvent.click(screen.getByText(/Resolve incident/));

    await waitFor(() => {
      expect(onResolve).toHaveBeenCalledWith('incident-1', {
        resolution_summary: 'replica destroyed',
        resolution_notes: { note: 'verified by compliance' },
      });
    });
  });
});
