'use client';
import { ChangeEvent } from 'react';

// key: lifecycle-console-ui -> filters
export interface LifecycleFilters {
  workspaceSearch: string;
  promotionLane: string;
  severity: string;
}

interface Props {
  filters: LifecycleFilters;
  onChange: (filters: LifecycleFilters) => void;
  onReset: () => void;
}

const LANE_OPTIONS = ['dev', 'pre-prod', 'candidate', 'production'];
const SEVERITY_OPTIONS = ['critical', 'high', 'medium', 'low'];

export default function LifecycleFilterBar({ filters, onChange, onReset }: Props) {
  const update = (patch: Partial<LifecycleFilters>) => {
    onChange({ ...filters, ...patch });
  };

  const handleInput = (event: ChangeEvent<HTMLInputElement | HTMLSelectElement>) => {
    const { name, value } = event.target;
    update({ [name]: value } as Partial<LifecycleFilters>);
  };

  return (
    <section className="rounded-lg border border-slate-200 bg-white shadow-sm p-4 space-y-3">
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <h2 className="text-sm font-semibold text-slate-700">Filters</h2>
        <button
          type="button"
          className="text-xs text-sky-700 font-medium hover:underline"
          onClick={() => onReset()}
        >
          Clear filters
        </button>
      </div>
      <div className="grid gap-3 md:grid-cols-3">
        <label className="text-xs font-medium text-slate-600 space-y-1">
          Workspace search
          <input
            type="text"
            name="workspaceSearch"
            value={filters.workspaceSearch}
            onChange={handleInput}
            placeholder="Search key or display name"
            className="w-full rounded border border-slate-200 px-2 py-1 text-sm focus:outline-none focus:ring-2 focus:ring-sky-500"
            autoComplete="off"
          />
        </label>
        <label className="text-xs font-medium text-slate-600 space-y-1">
          Promotion lane
          <div>
            <input
              type="text"
              name="promotionLane"
              value={filters.promotionLane}
              onChange={handleInput}
              list="lifecycle-promotion-lanes"
              placeholder="All lanes"
              className="w-full rounded border border-slate-200 px-2 py-1 text-sm focus:outline-none focus:ring-2 focus:ring-sky-500"
              autoComplete="off"
            />
            <datalist id="lifecycle-promotion-lanes">
              {LANE_OPTIONS.map((option) => (
                <option key={option} value={option} />
              ))}
            </datalist>
          </div>
        </label>
        <label className="text-xs font-medium text-slate-600 space-y-1">
          Severity
          <div>
            <input
              type="text"
              name="severity"
              value={filters.severity}
              onChange={handleInput}
              list="lifecycle-severity-options"
              placeholder="All severities"
              className="w-full rounded border border-slate-200 px-2 py-1 text-sm focus:outline-none focus:ring-2 focus:ring-sky-500"
              autoComplete="off"
            />
            <datalist id="lifecycle-severity-options">
              {SEVERITY_OPTIONS.map((option) => (
                <option key={option} value={option} />
              ))}
            </datalist>
          </div>
        </label>
      </div>
    </section>
  );
}
