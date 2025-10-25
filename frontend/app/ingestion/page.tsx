'use client';

/* musikconnect:
   purpose: Manage ingestion jobs for synchronizing data to vector DBs
   inputs: none (uses internal API calls)
   outputs: Ingestion jobs listing with create/delete actions
   status: experimental
   depends_on: ../lib/api, ../components/Button, ../components/Alert
   related_docs: ../../design-vision.md
*/
import { useState } from 'react';
import { useApi } from '../../lib/api';
import Button from '../../components/Button';
import Alert from '../../components/Alert';

interface Job {
  id: number;
  vector_db_id: number;
  source_url: string;
  schedule_minutes: number;
  last_run?: string;
}

interface Db { id: number; name: string; }

export default function IngestionPage() {
  const { data: jobs, isLoading, mutate } = useApi<Job[]>('/api/ingestion-jobs');
  const { data: dbs } = useApi<Db[]>('/api/vector-dbs');
  const [source, setSource] = useState('');
  const [dbId, setDbId] = useState<number>(0);
  const [schedule, setSchedule] = useState(60);
  const [error, setError] = useState('');

  const create = async () => {
    setError('');
    const res = await fetch('/api/ingestion-jobs', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ vector_db_id: dbId, source_url: source, schedule_minutes: schedule })
    });
    if (res.ok) {
      setSource('');
      setSchedule(60);
      mutate();
    } else {
      setError('Failed to create job');
    }
  };

  const del = async (id: number) => {
    await fetch(`/api/ingestion-jobs/${id}`, { method: 'DELETE', credentials: 'include' });
    mutate();
  };

  return (
    <div className="space-y-4 mt-6">
      <div className="flex gap-2 flex-wrap items-end">
        <select value={dbId} onChange={e => setDbId(Number(e.target.value))} className="border p-2">
          <option value={0}>Select DB</option>
          {dbs?.map(db => (
            <option key={db.id} value={db.id}>{db.name}</option>
          ))}
        </select>
        <input className="border p-2 flex-grow" placeholder="Source URL" value={source} onChange={e => setSource(e.target.value)} />
        <input className="border p-2 w-28" type="number" min="1" value={schedule} onChange={e => setSchedule(Number(e.target.value))} />
        <Button onClick={create}>Add Job</Button>
      </div>
      {error && <Alert message={error} />}
      {isLoading ? (<div>Loading...</div>) : (
        <ul className="space-y-2">
          {jobs?.map(job => (
            <li key={job.id} className="border p-2 rounded flex justify-between items-center">
              <div>
                <div className="font-medium">DB {job.vector_db_id} - {job.source_url}</div>
                <div className="text-sm text-gray-400">every {job.schedule_minutes}m{job.last_run && ` last run ${job.last_run}`}</div>
              </div>
              <Button variant="secondary" onClick={() => del(job.id)}>Delete</Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
