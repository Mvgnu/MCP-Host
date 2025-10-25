"use client";
import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import Spinner from '../../../../components/Spinner';

export default function ManifestPage({ params }: any) {
  const id = params.id;
  const [manifest, setManifest] = useState<any>(null);
  const [loading, setLoading] = useState(true);
  const router = useRouter();

  useEffect(() => {
    fetch(`/api/servers/${id}/manifest`, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error('failed');
        return res.json();
      })
      .then((data) => setManifest(data))
      .catch(() => setManifest(null))
      .finally(() => setLoading(false));
  }, [id]);

  return (
    <div className="space-y-4">
      <button className="px-2 py-1 bg-gray-600 text-white rounded" onClick={() => router.back()}>
        Back
      </button>
      {loading ? (
        <Spinner />
      ) : manifest ? (
        <pre className="whitespace-pre-wrap bg-gray-900 text-green-300 p-2 rounded">
          {JSON.stringify(manifest, null, 2)}
        </pre>
      ) : (
        <p>No manifest found.</p>
      )}
    </div>
  );
}
