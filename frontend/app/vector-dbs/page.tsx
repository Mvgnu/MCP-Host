'use client';
import { useState } from 'react';
import { useApi } from '../../lib/api';
import Card from '../../components/Card';
import Button from '../../components/Button';
import Alert from '../../components/Alert';

interface Db {
  id: number;
  name: string;
  db_type: string;
  url?: string;
}

export default function VectorDbPage() {
  const { data, isLoading, mutate } = useApi<Db[]>('/api/vector-dbs');
  const [name, setName] = useState('');
  const [error, setError] = useState('');

  const create = async () => {
    setError('');
    const res = await fetch('/api/vector-dbs', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name }),
    });
    if (res.ok) {
      setName('');
      mutate();
    } else {
      setError('Failed to create');
    }
  };

  const del = async (id: number) => {
    await fetch(`/api/vector-dbs/${id}`, { method: 'DELETE', credentials: 'include' });
    mutate();
  };

  return (
    <div className="space-y-4 mt-6">
      <div className="flex gap-2">
        <input value={name} onChange={e => setName(e.target.value)} className="border p-2 flex-grow" placeholder="Name" />
        <Button onClick={create}>Create</Button>
      </div>
      {error && <Alert message={error} />}
      {isLoading && <div>Loading...</div>}
      <div className="grid md:grid-cols-2 gap-4">
        {data?.map(db => (
          <Card key={db.id} className="flex justify-between items-center">
            <div>
              <div className="font-semibold">{db.name}</div>
              <div className="text-sm text-gray-400">{db.db_type}</div>
            </div>
            <Button onClick={() => del(db.id)} variant="secondary">Delete</Button>
          </Card>
        ))}
      </div>
    </div>
  );
}
