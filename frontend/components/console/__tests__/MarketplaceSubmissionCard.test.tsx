import { render, screen } from '@testing-library/react';
import MarketplaceSubmissionCard from '../MarketplaceSubmissionCard';
import type {
  ProviderMarketplaceEvaluationSummary,
  ProviderMarketplaceSubmission,
} from '../../../lib/marketplace';

describe('MarketplaceSubmissionCard', () => {
  function buildSubmission(overrides: Partial<ProviderMarketplaceSubmission> = {}): ProviderMarketplaceSubmission {
    return {
      id: 'sub-1',
      provider_id: 'prov-1',
      submitted_by: 7,
      tier: 'gold-inference',
      manifest_uri: 'oci://registry/model:1.0.0',
      artifact_digest: 'sha256:abc123',
      release_notes: 'Initial candidate',
      posture_state: {},
      posture_vetoed: false,
      posture_notes: [],
      status: 'pending',
      metadata: { version: '1.0.0' },
      created_at: '2025-12-18T10:00:00.000Z',
      updated_at: '2025-12-18T10:30:00.000Z',
      ...overrides,
    };
  }

  function buildEvaluations(): ProviderMarketplaceEvaluationSummary[] {
    return [
      {
        evaluation: {
          id: 'eval-1',
          submission_id: 'sub-1',
          evaluation_type: 'compliance',
          status: 'succeeded',
          started_at: '2025-12-18T11:00:00.000Z',
          completed_at: '2025-12-18T11:30:00.000Z',
          evaluator_ref: 'automation',
          result: { score: 'pass' },
          posture_state: {},
          posture_vetoed: false,
          posture_notes: ['eligible'],
          created_at: '2025-12-18T11:00:00.000Z',
          updated_at: '2025-12-18T11:30:00.000Z',
        },
        promotions: [
          {
            id: 'promo-1',
            evaluation_id: 'eval-1',
            gate: 'sandbox',
            status: 'approved',
            opened_at: '2025-12-18T12:00:00.000Z',
            closed_at: '2025-12-18T13:00:00.000Z',
            notes: ['ready'],
            created_at: '2025-12-18T12:00:00.000Z',
            updated_at: '2025-12-18T13:00:00.000Z',
          },
        ],
      },
    ];
  }

  it('renders submission context with posture badge', () => {
    render(
      <MarketplaceSubmissionCard submission={buildSubmission()} evaluations={buildEvaluations()} />,
    );

    expect(screen.getByText('oci://registry/model:1.0.0')).toBeInTheDocument();
    expect(
      screen.getByText((content, element) => element?.textContent === 'Tier gold-inference · Status pending'),
    ).toBeInTheDocument();
    expect(screen.getByText(/Posture healthy/i)).toBeInTheDocument();
  });

  it('lists evaluations and promotion notes', () => {
    render(
      <MarketplaceSubmissionCard submission={buildSubmission()} evaluations={buildEvaluations()} />,
    );

    expect(screen.getByText(/compliance/i)).toBeInTheDocument();
    expect(screen.getByText(/eligible/)).toBeInTheDocument();
    expect(screen.getByText(/Promotion gates/i)).toBeInTheDocument();
    expect(screen.getByText('ready')).toBeInTheDocument();
  });

  it('highlights posture veto when submission is blocked', () => {
    render(
      <MarketplaceSubmissionCard
        submission={buildSubmission({ posture_vetoed: true, posture_notes: ['missing attestation'] })}
        evaluations={[]}
      />,
    );

    expect(screen.getByText(/Posture vetoed/i)).toBeInTheDocument();
    expect(screen.getByText(/Posture notes/i)).toBeInTheDocument();
    expect(screen.getByText(/missing attestation/)).toBeInTheDocument();
  });
});
