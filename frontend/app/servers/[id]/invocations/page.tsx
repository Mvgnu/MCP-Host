"use client";
import { useEffect, useState } from 'react';
import Spinner from '../../../../components/Spinner';

interface Trace { id: number; input_json: any; output_text?: string; created_at: string; }

export default function InvocationsPage({ params }: any) {
  const id = params.id;
  const [traces, setTraces] = useState<Trace[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`/api/servers/${id}/invocations`, { credentials: 'include' })
      .then((res) => { if (!res.ok) throw new Error('failed'); return res.json(); })
      .then(setTraces)
      .catch(() => setTraces([]))
      .finally(() => setLoading(false));
  }, [id]);

  return (
    <div className="space-y-4">
      {loading ? (
        <Spinner />
      ) : traces.length > 0 ? (
        <ul className="space-y-2">
          {traces.map((t) => (
            <li key={t.id} className="bg-gray-900 p-2 rounded">
              <div className="text-xs text-gray-400">{t.created_at}</div>
              <pre className="whitespace-pre-wrap text-green-300">{JSON.stringify(t.input_json)}</pre>
              {t.output_text && (
                <pre className="whitespace-pre-wrap mt-1 text-blue-200">{t.output_text}</pre>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p>No invocations recorded.</p>
      )}
    </div>
  );
}
