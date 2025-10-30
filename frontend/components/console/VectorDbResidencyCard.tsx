'use client';
import { useCallback, useMemo, useState } from 'react';
import Button from '../Button';
import Input from '../Input';
import Spinner from '../Spinner';
import type {
  UpsertVectorDbResidencyPolicyPayload,
  VectorDbResidencyPolicy,
} from '../../lib/vectorDbs';

// key: vector-dbs-console -> residency-card
interface VectorDbResidencyCardProps {
  policies: VectorDbResidencyPolicy[];
  onUpsert: (payload: UpsertVectorDbResidencyPolicyPayload) => Promise<void> | void;
  loading?: boolean;
  error?: string | null;
}

interface ResidencyFormState {
  region: string;
  dataClassification: string;
  enforcementMode: string;
  active: boolean;
}

export default function VectorDbResidencyCard({
  policies,
  onUpsert,
  loading = false,
  error = null,
}: VectorDbResidencyCardProps) {
  const [formState, setFormState] = useState<ResidencyFormState>({
    region: '',
    dataClassification: 'general',
    enforcementMode: 'monitor',
    active: true,
  });
  const [formError, setFormError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  const sortedPolicies = useMemo(
    () =>
      policies
        .slice()
        .sort((a, b) => a.region.localeCompare(b.region)),
    [policies],
  );

  const resetForm = useCallback(() => {
    setFormState({ region: '', dataClassification: 'general', enforcementMode: 'monitor', active: true });
  }, []);

  const handleChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    const { name, value, type, checked } = event.target;
    setFormState((current) => ({
      ...current,
      [name]: type === 'checkbox' ? checked : value,
    }));
  }, []);

  const handleSubmit = useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const region = formState.region.trim();
      if (!region) {
        setFormError('Region is required to define a residency policy.');
        return;
      }
      setIsSubmitting(true);
      setFormError(null);
      setSuccessMessage(null);
      try {
        await onUpsert({
          region,
          data_classification: formState.dataClassification.trim() || undefined,
          enforcement_mode: formState.enforcementMode.trim() || undefined,
          active: formState.active,
        });
        setSuccessMessage(`Residency policy for ${region} saved.`);
        resetForm();
      } catch (cause) {
        console.error('failed to upsert residency policy', cause);
        setFormError(cause instanceof Error ? cause.message : 'Failed to save residency policy');
      } finally {
        setIsSubmitting(false);
      }
    },
    [formState, onUpsert, resetForm],
  );

  return (
    <section className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-4">
      <header className="space-y-1">
        <h2 className="text-lg font-semibold text-slate-800">Residency policies</h2>
        <p className="text-sm text-slate-600">
          Enforce regional data handling requirements and monitor residency posture across attachments.
        </p>
      </header>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-slate-600">
          <Spinner size="sm" /> Loading residency policies…
        </div>
      ) : sortedPolicies.length === 0 ? (
        <p className="text-sm text-slate-500">No residency policies defined yet.</p>
      ) : (
        <ul className="space-y-3">
          {sortedPolicies.map((policy) => (
            <li key={policy.id} className="border border-slate-100 rounded p-3 bg-slate-50">
              <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-2">
                <div>
                  <p className="text-sm font-semibold text-slate-700">{policy.region}</p>
                  <p className="text-xs text-slate-500">
                    {policy.data_classification} · {policy.enforcement_mode}
                  </p>
                </div>
                <span
                  className={
                    policy.active
                      ? 'inline-flex items-center px-2 py-1 text-xs font-semibold rounded-full bg-emerald-100 text-emerald-700'
                      : 'inline-flex items-center px-2 py-1 text-xs font-semibold rounded-full bg-slate-200 text-slate-600'
                  }
                >
                  {policy.active ? 'Active' : 'Inactive'}
                </span>
              </div>
              <p className="mt-2 text-xs text-slate-500">
                Updated {new Date(policy.updated_at).toLocaleString()} · Created{' '}
                {new Date(policy.created_at).toLocaleString()}
              </p>
            </li>
          ))}
        </ul>
      )}

      <form onSubmit={handleSubmit} className="space-y-3 border-t border-slate-200 pt-3">
        <h3 className="text-sm font-semibold text-slate-700">Add or update residency policy</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          <Input
            label="Region"
            name="region"
            value={formState.region}
            onChange={handleChange}
            placeholder="us-east"
            required
          />
          <Input
            label="Data classification"
            name="dataClassification"
            value={formState.dataClassification}
            onChange={handleChange}
            placeholder="general"
          />
          <Input
            label="Enforcement mode"
            name="enforcementMode"
            value={formState.enforcementMode}
            onChange={handleChange}
            placeholder="monitor"
          />
          <label className="flex items-center gap-2 text-sm text-slate-700 mt-2">
            <input
              type="checkbox"
              name="active"
              checked={formState.active}
              onChange={handleChange}
            />
            Active
          </label>
        </div>
        {formError && <p className="text-sm text-red-600">{formError}</p>}
        {error && <p className="text-sm text-red-600">{error}</p>}
        {successMessage && <p className="text-sm text-emerald-600">{successMessage}</p>}
        <div className="flex items-center gap-3">
          <Button disabled={isSubmitting}>{isSubmitting ? 'Saving…' : 'Save policy'}</Button>
          <button
            type="button"
            onClick={resetForm}
            disabled={isSubmitting}
            className="inline-block px-5 py-2 rounded font-medium border border-slate-200 text-slate-700 bg-white hover:bg-slate-50 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Clear
          </button>
        </div>
      </form>
    </section>
  );
}
