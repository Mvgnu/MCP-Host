'use client';
import { useState, useEffect } from 'react';
import Spinner from '../../../../components/Spinner';
import Alert from '../../../../components/Alert';

export default function DomainsPage({ params }: any) {
  const id = params.id;
  const [domains, setDomains] = useState<any[]>([]);
  const [domain, setDomain] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchDomains = () => {
    fetch(`/api/servers/${id}/domains`, { credentials: 'include' })
      .then(res => res.json())
      .then(setDomains)
      .catch(() => setError('Failed to load domains'));
  };

  useEffect(() => {
    fetchDomains();
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError(null);
    const res = await fetch(`/api/servers/${id}/domains`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ domain })
    });
    setLoading(false);
    if (res.ok) {
      setDomain('');
      fetchDomains();
    } else {
      setError(await res.text());
    }
  };

  const remove = async (domId: number) => {
    setLoading(true);
    setError(null);
    const res = await fetch(`/api/servers/${id}/domains/${domId}`, {
      method: 'DELETE',
      credentials: 'include'
    });
    setLoading(false);
    if (res.ok) {
      fetchDomains();
    } else {
      setError(await res.text());
    }
  };

  return (
    <div className="p-4 max-w-md mx-auto space-y-4">
      <h1 className="text-xl font-semibold">Custom Domains</h1>
      {error && <Alert message={error} />}
      <ul className="space-y-2">
        {domains.map(d => (
          <li key={d.id} className="border p-2 rounded flex justify-between items-center">
            <span>{d.domain}</span>
            <button className="px-2 py-1 bg-red-600 text-white rounded" onClick={() => remove(d.id)}>Delete</button>
          </li>
        ))}
      </ul>
      <form onSubmit={handleSubmit} className="space-y-2">
        <input value={domain} onChange={e => setDomain(e.target.value)} placeholder="example.com" className="border p-2 w-full" />
        <button type="submit" disabled={loading} className="bg-blue-600 text-white p-2 w-full flex justify-center">
          {loading ? <Spinner /> : 'Add Domain'}
        </button>
      </form>
    </div>
  );
}
