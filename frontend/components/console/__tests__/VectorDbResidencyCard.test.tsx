import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import VectorDbResidencyCard from '../VectorDbResidencyCard';
import type { VectorDbResidencyPolicy } from '../../../lib/vectorDbs';

describe('VectorDbResidencyCard', () => {
  const policies: VectorDbResidencyPolicy[] = [
    {
      id: 1,
      vector_db_id: 44,
      region: 'us-east',
      data_classification: 'restricted',
      enforcement_mode: 'block',
      active: true,
      created_at: '2025-12-20T12:00:00.000Z',
      updated_at: '2025-12-20T12:30:00.000Z',
    },
  ];

  it('renders residency policy list', () => {
    render(
      <VectorDbResidencyCard policies={policies} onUpsert={jest.fn()} />,
    );

    expect(screen.getByText('Residency policies')).toBeInTheDocument();
    expect(screen.getByText('us-east')).toBeInTheDocument();
    expect(screen.getByText(/restricted/)).toBeInTheDocument();
  });

  it('submits new residency policy details', async () => {
    const onUpsert = jest.fn().mockResolvedValue(undefined);

    render(
      <VectorDbResidencyCard policies={policies} onUpsert={onUpsert} />,
    );

    fireEvent.change(screen.getByLabelText('Region'), { target: { value: 'eu-central' } });
    fireEvent.change(screen.getByLabelText('Data classification'), { target: { value: 'confidential' } });
    fireEvent.change(screen.getByLabelText('Enforcement mode'), { target: { value: 'monitor' } });
    fireEvent.click(screen.getByRole('checkbox', { name: /active/i }));
    fireEvent.click(screen.getByText(/Save policy/i));

    await waitFor(() => {
      expect(onUpsert).toHaveBeenCalledWith({
        region: 'eu-central',
        data_classification: 'confidential',
        enforcement_mode: 'monitor',
        active: false,
      });
    });
  });
});
