'use client';
import { useMemo } from 'react';
import { LifecycleRunArtifact, LifecycleRunSnapshot } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> run-progress
interface Props {
  run: LifecycleRunSnapshot;
  onSelect?: (run: LifecycleRunSnapshot) => void;
}

function formatDurationSeconds(durationSeconds?: number | null) {
  if (typeof durationSeconds !== 'number' || Number.isNaN(durationSeconds) || durationSeconds < 0) {
    return undefined;
  }
  if (durationSeconds < 60) {
    return `${Math.floor(durationSeconds)}s`;
  }
  const minutes = Math.floor(durationSeconds / 60);
  const seconds = Math.floor(durationSeconds % 60);
  if (minutes < 60) {
    return seconds === 0 ? `${minutes}m` : `${minutes}m ${seconds}s`;
  }
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  return remMinutes === 0 ? `${hours}h` : `${hours}h ${remMinutes}m`;
}

function formatDurationMs(durationMs?: number | null) {
  if (typeof durationMs !== 'number' || Number.isNaN(durationMs) || durationMs < 0) {
    return undefined;
  }
  return formatDurationSeconds(durationMs / 1000);
}

function fallbackDuration(startedAt: string, completedAt?: string | null) {
  const started = new Date(startedAt).getTime();
  const finished = completedAt ? new Date(completedAt).getTime() : Date.now();
  if (Number.isNaN(started) || Number.isNaN(finished)) return undefined;
  const seconds = Math.max(0, Math.floor((finished - started) / 1000));
  return formatDurationSeconds(seconds);
}

function summarizeAttempt(attempt?: number | null, retryLimit?: number | null) {
  if (typeof attempt !== 'number' && typeof retryLimit !== 'number') {
    return undefined;
  }
  const attemptPart = typeof attempt === 'number' ? String(Math.floor(attempt)) : '–';
  if (typeof retryLimit !== 'number') {
    return attemptPart;
  }
  return `${attemptPart}/${Math.floor(retryLimit)}`;
}

function summarizeRetryCount(count?: number | null) {
  if (typeof count !== 'number' || Number.isNaN(count) || count < 0) {
    return undefined;
  }
  if (count === 1) {
    return '1 attempt logged';
  }
  return `${Math.floor(count)} attempts logged`;
}

function summarizeRetryLedger(run: LifecycleRunSnapshot) {
  const ledger = run.retry_ledger ?? [];
  if (ledger.length === 0) {
    return undefined;
  }
  const latest = ledger[ledger.length - 1];
  const observed = latest.observed_at ? new Date(latest.observed_at).toLocaleString() : undefined;
  const pieces: string[] = [`latest #${Math.floor(latest.attempt)}`];
  if (latest.status) pieces.push(latest.status);
  if (latest.reason) pieces.push(`reason:${latest.reason}`);
  if (observed) pieces.push(observed);
  return pieces.join(' · ');
}

function summarizeArtifact(artifact: LifecycleRunArtifact) {
  const digest = artifact.manifest_digest;
  const shortDigest = digest.length > 12 ? `${digest.slice(0, 12)}…` : digest;
  const parts: string[] = [];
  if (artifact.lane) parts.push(`lane:${artifact.lane}`);
  if (artifact.stage) parts.push(`stage:${artifact.stage}`);
  if (artifact.manifest_tag) parts.push(`tag:${artifact.manifest_tag}`);
  if (artifact.registry_image) parts.push(`image:${artifact.registry_image}`);
  if (artifact.build_status) parts.push(`build:${artifact.build_status}`);
  if (typeof artifact.duration_seconds === 'number') {
    const formatted = formatDurationSeconds(artifact.duration_seconds);
    if (formatted) parts.push(`duration:${formatted}`);
  }
  return parts.length > 0 ? `${shortDigest} (${parts.join(', ')})` : shortDigest;
}

function summarizeOverride(run: LifecycleRunSnapshot) {
  const override = run.manual_override;
  if (!override) {
    return run.override_reason ?? undefined;
  }
  const pieces = [override.reason];
  if (override.actor_email) {
    pieces.push(`actor:${override.actor_email}`);
  } else if (typeof override.actor_id === 'number') {
    pieces.push(`actor-id:${override.actor_id}`);
  }
  return pieces.join(' · ');
}

function summarizeFingerprints(run: LifecycleRunSnapshot) {
  const fingerprints = run.artifact_fingerprints ?? [];
  if (fingerprints.length === 0) {
    return undefined;
  }
  return fingerprints
    .map((entry) => `${entry.manifest_digest.slice(0, 12)}…=${entry.fingerprint.slice(0, 16)}…`)
    .join(', ');
}

function summarizePromotion(verdict?: LifecycleRunSnapshot['promotion_verdict']) {
  if (!verdict) {
    return undefined;
  }
  const parts: string[] = [];
  parts.push(`verdict #${verdict.verdict_id}`);
  if (typeof verdict.allowed === 'boolean') {
    parts.push(verdict.allowed ? 'allowed' : 'blocked');
  }
  if (verdict.stage) parts.push(`stage:${verdict.stage}`);
  if (verdict.track_name) parts.push(`track:${verdict.track_name}`);
  if (verdict.track_tier) parts.push(`tier:${verdict.track_tier}`);
  return parts.join(' · ');
}

function summarizeProviderKey(posture?: LifecycleRunSnapshot['provider_key_posture']) {
  if (!posture) {
    return undefined;
  }
  const parts: string[] = [];
  if (posture.state) {
    parts.push(posture.state.replace(/_/g, ' '));
  }
  if (!posture.attestation_registered) {
    parts.push('attestation-missing');
  } else if (!posture.attestation_signature_verified) {
    parts.push('signature-unverified');
  }
  parts.push(posture.vetoed ? 'vetoed' : 'clear');
  if (posture.rotation_due_at) {
    const deadline = new Date(posture.rotation_due_at);
    if (!Number.isNaN(deadline.getTime())) {
      parts.push(`rotation:${deadline.toLocaleDateString()}`);
    }
  }
  if (posture.notes && posture.notes.length > 0) {
    const preview = posture.notes.slice(0, 2).join('|');
    parts.push(`notes:${preview}`);
  }
  return parts.join(' · ');
}

export default function LifecycleRunProgress({ run, onSelect }: Props) {
  const statusBadge = useMemo(() => {
    const status = run.run.status;
    const base = 'px-2 py-1 rounded text-xs font-semibold';
    switch (status) {
      case 'succeeded':
        return `${base} bg-emerald-100 text-emerald-700`;
      case 'failed':
        return `${base} bg-rose-100 text-rose-700`;
      case 'cancelled':
        return `${base} bg-amber-100 text-amber-700`;
      default:
        return `${base} bg-sky-100 text-sky-700`;
    }
  }, [run.run.status]);

  const startedAt = run.execution_window?.started_at ?? run.run.started_at;
  const completedAt = run.execution_window?.completed_at ?? run.run.completed_at;
  const durationDisplay =
    formatDurationMs(run.duration_ms ?? null) ??
    formatDurationSeconds(run.duration_seconds ?? null) ??
    fallbackDuration(startedAt, completedAt);
  const providerKeySummary = summarizeProviderKey(run.provider_key_posture);

  return (
    <div className="border border-slate-200 rounded p-3 bg-white shadow-sm">
      <div className="flex items-center justify-between gap-2">
        <div>
          <p className="text-sm font-semibold">Run #{run.run.id}</p>
          <p className="text-xs text-slate-500">
            Started {new Date(startedAt).toLocaleString()} ·
            {durationDisplay ? ` ${durationDisplay}` : ' unknown duration'}
          </p>
        </div>
        <span className={statusBadge}>{run.run.status}</span>
        {onSelect && (
          <button
            type="button"
            className="px-2 py-1 text-xs font-medium rounded border border-slate-200 text-slate-600 hover:bg-slate-100 focus:outline-none focus:ring-2 focus:ring-sky-500"
            onClick={() => onSelect(run)}
          >
            View details
          </button>
        )}
      </div>
      {run.run.failure_reason && (
        <p className="mt-2 text-xs text-rose-600">
          Failure: {run.run.failure_reason}
        </p>
      )}
      {run.run.approval_required && (
        <p className="mt-2 text-xs text-amber-600">
          Awaiting approval – state: {run.run.approval_state ?? 'pending'}
        </p>
      )}
      <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-slate-600">
        {summarizeAttempt(run.retry_attempt ?? null, run.retry_limit ?? null) && (
          <span className="rounded border border-slate-200 bg-slate-50 px-2 py-0.5">
            Attempt {summarizeAttempt(run.retry_attempt ?? null, run.retry_limit ?? null)}
          </span>
        )}
        {summarizeRetryCount(run.retry_count ?? null) && (
          <span className="rounded border border-slate-200 bg-slate-50 px-2 py-0.5">
            {summarizeRetryCount(run.retry_count ?? null)}
          </span>
        )}
        {summarizeRetryLedger(run) && (
          <span className="rounded border border-slate-200 bg-slate-50 px-2 py-0.5">
            {summarizeRetryLedger(run)}
          </span>
        )}
        {run.override_reason && (
          <span className="rounded border border-amber-200 bg-amber-50 px-2 py-0.5 text-amber-700">
            Override: {run.override_reason}
          </span>
        )}
        {summarizeOverride(run) && summarizeOverride(run) !== run.override_reason && (
          <span className="rounded border border-amber-200 bg-amber-50 px-2 py-0.5 text-amber-700">
            {summarizeOverride(run)}
          </span>
        )}
        {summarizePromotion(run.promotion_verdict) && (
          <span className="rounded border border-emerald-200 bg-emerald-50 px-2 py-0.5 text-emerald-700">
            {summarizePromotion(run.promotion_verdict)}
          </span>
        )}
        {providerKeySummary && run.provider_key_posture && (
          <span
            className={`rounded border px-2 py-0.5 ${
              run.provider_key_posture.vetoed
                ? 'border-rose-200 bg-rose-50 text-rose-600'
                : 'border-emerald-200 bg-emerald-50 text-emerald-700'
            }`}
          >
            BYOK {providerKeySummary}
          </span>
        )}
        {summarizeFingerprints(run) && (
          <span className="rounded border border-slate-200 bg-slate-50 px-2 py-0.5">
            Fingerprints: {summarizeFingerprints(run)}
          </span>
        )}
        {(run.artifacts ?? []).map((artifact) => (
          <span
            key={`${run.run.id}-${artifact.manifest_digest}-${artifact.manifest_tag ?? ''}`}
            className="rounded border border-sky-200 bg-sky-50 px-2 py-0.5 text-sky-700"
          >
            {summarizeArtifact(artifact)}
          </span>
        ))}
      </div>
    </div>
  );
}
