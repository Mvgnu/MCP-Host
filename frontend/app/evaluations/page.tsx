'use client';
import { useEffect, useState } from 'react';
import Alert from '../../components/Alert';
import Card from '../../components/Card';

interface Row { server: string; average_score: number; runs: number; }

export default function EvaluationsPage() {
  const [rows, setRows] = useState<Row[]>([]);
  const [error, setError] = useState('');

  useEffect(() => {
    fetch('/api/evaluations/summary', { credentials: 'include' })
      .then(r => (r.ok ? r.json() : Promise.reject('failed')))
      .then(setRows)
      .catch(() => setError('Failed to load results'));
  }, []);

  return (
    <div className="mt-6 space-y-4">
      <h1 className="text-xl font-semibold">Server Evaluation Scores</h1>
      {error && <Alert message={error} />}
      <Card>
        <table className="w-full text-left border-collapse">
        <thead>
          <tr className="border-b">
            <th className="p-2">Server</th>
            <th className="p-2">Average Score</th>
            <th className="p-2">Runs</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} className="border-b">
              <td className="p-2">{r.server}</td>
              <td className="p-2">{r.average_score.toFixed(2)}</td>
              <td className="p-2">{r.runs}</td>
            </tr>
          ))}
        </tbody>
      </table>
      </Card>
    </div>
  );
}
