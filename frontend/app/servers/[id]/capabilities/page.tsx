"use client";
import { useEffect, useState } from 'react';
import Spinner from '../../../../components/Spinner';

export default function CapabilitiesPage({ params }: any) {
  const id = params.id;
  const [caps, setCaps] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch(`/api/servers/${id}/capabilities`, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('failed');
        return res.json();
      })
      .then(setCaps)
      .catch(() => setCaps([]))
      .finally(() => setLoading(false));
  }, [id]);

  return (
    <div className="space-y-4">
      {loading ? (
        <Spinner />
      ) : caps.length > 0 ? (
        <ul className="list-disc pl-6 space-y-1">
          {caps.map((c) => (
            <li key={c.id}>
              <span className="font-semibold">{c.name}:</span> {c.description || ''}
            </li>
          ))}
        </ul>
      ) : (
        <p>No capabilities found.</p>
      )}
    </div>
  );
}
