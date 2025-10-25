import { Line } from 'react-chartjs-2';
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend,
} from 'chart.js';
import { useMemo, useState, useEffect } from 'react';

ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend
);

interface MetricEvent {
  id: number;
  timestamp: string;
  event_type: string;
  details?: any;
}

const FRIENDLY_LABELS: Record<string, string> = {
  tag_started: 'Tagging started',
  tag_succeeded: 'Tagging completed',
  push_started: 'Push started',
  push_retry: 'Push retry scheduled',
  push_succeeded: 'Push completed',
  push_failed: 'Push failed',
};

export default function MetricsChart({ events }: { events: MetricEvent[] }) {
  const eventTypes = useMemo(() => {
    const types = Array.from(new Set(events.map((e) => e.event_type)));
    types.sort();
    return types;
  }, [events]);
  const [selected, setSelected] = useState<string[]>(eventTypes);

  // update selected types when events change
  useEffect(() => {
    setSelected((sel) => sel.filter((t) => eventTypes.includes(t)));
  }, [eventTypes]);

  const colors = ['#60a5fa', '#10b981', '#f97316', '#e11d48', '#a855f7'];

  const data = useMemo(() => {
    const labels = events
      .map((e) => new Date(e.timestamp).toLocaleTimeString())
      .reverse();

    const datasets = eventTypes
      .filter((t) => selected.includes(t))
      .map((type, idx) => {
        const cumulative: number[] = [];
        let count = 0;
        events
          .slice()
          .reverse()
          .forEach((e) => {
            if (e.event_type === type) count += 1;
            cumulative.push(count);
          });
        cumulative.reverse();
        const color = colors[idx % colors.length];
        const label = FRIENDLY_LABELS[type] ?? type;
        return {
          label,
          data: cumulative,
          borderColor: color,
          backgroundColor: `${color}33`,
        };
      });

    return { labels, datasets };
  }, [events, eventTypes, selected]);

  const options = useMemo(
    () => ({
      responsive: true,
      plugins: {
        legend: {
          position: 'bottom' as const,
          labels: { color: '#fff' },
        },
      },
      scales: {
        x: {
          ticks: { color: '#fff' },
          grid: { color: '#444' },
        },
        y: {
          beginAtZero: true,
          ticks: { stepSize: 1, color: '#fff' },
          grid: { color: '#444' },
        },
      },
    }),
    []
  );

  return (
    <div>
      <div className="flex space-x-3 mb-2 text-sm text-white">
        {eventTypes.map((t) => (
          <label key={t} className="flex items-center space-x-1">
            <input
              type="checkbox"
              checked={selected.includes(t)}
              onChange={() =>
                setSelected((sel) =>
                  sel.includes(t) ? sel.filter((x) => x !== t) : [...sel, t]
                )
              }
            />
            <span>
              {FRIENDLY_LABELS[t] ?? t}
              {FRIENDLY_LABELS[t] ? (
                <span className="ml-1 text-xs text-slate-400">({t})</span>
              ) : null}
            </span>
          </label>
        ))}
      </div>
      <Line options={options} data={data} />
    </div>
  );
}
