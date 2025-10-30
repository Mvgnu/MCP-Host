'use client';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import Alert from '../../../../components/Alert';
import Button from '../../../../components/Button';
import Input from '../../../../components/Input';
import Spinner from '../../../../components/Spinner';
import Textarea from '../../../../components/Textarea';
import { MarketplaceSubmissionCard } from '../../../../components/console';
import type { ProviderMarketplaceSubmissionSummary } from '../../../../lib/marketplace';
import {
  createProviderSubmission,
  fetchProviderSubmissions,
  openMarketplaceEventStream,
} from '../../../../lib/marketplace';

// key: marketplace-console-page -> provider-dashboard
interface SubmissionFormState {
  tier: string;
  manifestUri: string;
  artifactDigest: string;
  releaseNotes: string;
}

export default function ProviderMarketplaceDashboardPage() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const initialProviderId = searchParams.get('providerId') ?? '';
  const [providerInput, setProviderInput] = useState(initialProviderId);
  const [providerId, setProviderId] = useState(initialProviderId);
  const [submissions, setSubmissions] = useState<ProviderMarketplaceSubmissionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [formState, setFormState] = useState<SubmissionFormState>({
    tier: '',
    manifestUri: '',
    artifactDigest: '',
    releaseNotes: '',
  });

  const loadSubmissions = useCallback(
    async (showSpinner: boolean) => {
      if (!providerId) {
        setSubmissions([]);
        return;
      }
      if (showSpinner) {
        setLoading(true);
      }
      try {
        setError(null);
        const records = await fetchProviderSubmissions(providerId);
        setSubmissions(records);
      } catch (cause) {
        console.error('failed to load marketplace submissions', cause);
        setError(cause instanceof Error ? cause.message : 'Failed to load submissions');
      } finally {
        if (showSpinner) {
          setLoading(false);
        }
      }
    },
    [providerId],
  );

  useEffect(() => {
    if (!providerId) {
      return;
    }
    loadSubmissions(true);
  }, [providerId, loadSubmissions]);

  useEffect(() => {
    if (!providerId) {
      return;
    }
    const unsubscribe = openMarketplaceEventStream(providerId, () => {
      loadSubmissions(false);
    });
    return () => {
      unsubscribe();
    };
  }, [providerId, loadSubmissions]);

  const sortedSubmissions = useMemo(
    () =>
      submissions
        .slice()
        .sort(
          (a, b) =>
            new Date(b.submission.created_at).getTime() - new Date(a.submission.created_at).getTime(),
        ),
    [submissions],
  );

  const handleSelectProvider = useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmed = providerInput.trim();
      setSuccessMessage(null);
      setFormError(null);
      setError(null);
      if (!trimmed) {
        setProviderId('');
        setSubmissions([]);
        router.replace('?');
        return;
      }
      setProviderId(trimmed);
      const params = new URLSearchParams(window.location.search);
      params.set('providerId', trimmed);
      router.replace(`?${params.toString()}`);
    },
    [providerInput, router],
  );

  const handleSubmissionChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      const { name, value } = event.target;
      setFormState((current) => ({ ...current, [name]: value }));
    },
    [],
  );

  const handleCreateSubmission = useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!providerId) {
        setFormError('Select a provider before uploading artifacts.');
        return;
      }
      const tier = formState.tier.trim();
      const manifestUri = formState.manifestUri.trim();
      if (!tier || !manifestUri) {
        setFormError('Tier and manifest URI are required.');
        return;
      }
      setIsSubmitting(true);
      setFormError(null);
      setSuccessMessage(null);
      try {
        await createProviderSubmission(providerId, {
          tier,
          manifest_uri: manifestUri,
          artifact_digest: formState.artifactDigest.trim() || undefined,
          release_notes: formState.releaseNotes.trim() || undefined,
        });
        setSuccessMessage('Submission queued for evaluation.');
        setFormState({ tier: '', manifestUri: '', artifactDigest: '', releaseNotes: '' });
        await loadSubmissions(false);
      } catch (cause) {
        console.error('failed to create marketplace submission', cause);
        setFormError(cause instanceof Error ? cause.message : 'Failed to create submission');
      } finally {
        setIsSubmitting(false);
      }
    },
    [formState, loadSubmissions, providerId],
  );

  return (
    <div className="space-y-6">
      <header className="space-y-2">
        <h1 className="text-2xl font-semibold text-slate-900">Provider marketplace dashboard</h1>
        <p className="text-sm text-slate-600">
          Upload artifacts, monitor evaluation progress, and track promotion gates without leaving the console.
        </p>
      </header>

      <section className="border border-slate-200 rounded-lg p-4 space-y-4 bg-white shadow-sm">
        <form className="space-y-3" onSubmit={handleSelectProvider}>
          <Input
            label="Provider ID"
            name="providerId"
            value={providerInput}
            onChange={(event) => setProviderInput(event.target.value)}
            placeholder="00000000-0000-0000-0000-000000000000"
            required
          />
          <Button type="submit">Load provider workspace</Button>
        </form>
        <p className="text-xs text-slate-500">
          Only providers in good BYOK posture can submit artifacts. Use the loader above to switch between providers.
        </p>
      </section>

      <section className="border border-slate-200 rounded-lg p-4 space-y-4 bg-white shadow-sm">
        <h2 className="text-lg font-semibold text-slate-800">Submit artifact for evaluation</h2>
        {formError && <Alert message={formError} type="error" />}
        {successMessage && <Alert message={successMessage} type="success" />}
        <form className="grid gap-4 md:grid-cols-2" onSubmit={handleCreateSubmission}>
          <Input
            label="Tier"
            name="tier"
            value={formState.tier}
            onChange={handleSubmissionChange}
            placeholder="gold-inference"
            required
          />
          <Input
            label="Artifact digest"
            name="artifactDigest"
            value={formState.artifactDigest}
            onChange={handleSubmissionChange}
            placeholder="sha256:..."
          />
          <div className="md:col-span-2">
            <Input
              label="Manifest URI"
              name="manifestUri"
              value={formState.manifestUri}
              onChange={handleSubmissionChange}
              placeholder="oci://registry/image:tag"
              required
            />
          </div>
          <div className="md:col-span-2">
            <Textarea
              label="Release notes"
              name="releaseNotes"
              value={formState.releaseNotes}
              onChange={handleSubmissionChange}
              placeholder="Summarize the change, compliance evidence, or validation context."
              rows={4}
            />
          </div>
          <div className="md:col-span-2 flex items-center gap-3">
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting ? 'Submitting…' : 'Create submission'}
            </Button>
            <p className="text-xs text-slate-500">Submissions immediately trigger posture checks and evaluation runs.</p>
          </div>
        </form>
      </section>

      {!providerId && (
        <Alert message="Enter a provider ID to load marketplace submissions." type="error" />
      )}

      {error && <Alert message={error} type="error" />}

      {loading ? (
        <div className="flex items-center gap-2 text-slate-600">
          <Spinner />
          <span>Loading submissions…</span>
        </div>
      ) : (
        <div className="space-y-4">
          {sortedSubmissions.length === 0 ? (
            <p className="text-sm text-slate-500">
              {providerId
                ? 'No submissions recorded for this provider yet. Use the form above to submit artifacts.'
                : 'Set a provider to review submissions.'}
            </p>
          ) : (
            sortedSubmissions.map((summary) => (
              <MarketplaceSubmissionCard
                key={summary.submission.id}
                submission={summary.submission}
                evaluations={summary.evaluations}
              />
            ))
          )}
        </div>
      )}
    </div>
  );
}
