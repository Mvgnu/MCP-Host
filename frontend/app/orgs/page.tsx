'use client'
import { useState } from 'react';
import { useApi } from '../../lib/api';
import Card from '../../components/Card';
import Button from '../../components/Button';
import Alert from '../../components/Alert';

interface Org { id: number; name: string; }

export default function OrgsPage() {
  const { data, isLoading, mutate } = useApi<Org[]>('/api/orgs');
  const [name, setName] = useState('');
  const [error, setError] = useState('');

  const create = async () => {
    setError('');
    const res = await fetch('/api/orgs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ name }),
    });
    if (res.ok) {
      setName('');
      mutate();
    } else {
      setError('Failed to create');
    }
  };

  return (
    <div className="space-y-4 mt-6">
      <div className="flex gap-2">
        <input
          value={name}
          onChange={e => setName(e.target.value)}
          className="border p-2 flex-grow"
          placeholder="Organization name"
        />
        <Button onClick={create}>Create</Button>
      </div>
      {error && <Alert message={error} />}
      {isLoading && <div>Loading...</div>}
      <div className="grid md:grid-cols-2 gap-4">
        {data?.map(org => (
          <Card key={org.id}>{org.name}</Card>
        ))}
      </div>
    </div>
  );
}
