"use client";
import { useEffect, useState } from 'react';
import Spinner from '../../../../components/Spinner';
import Button from '../../../../components/Button';
import Alert from '../../../../components/Alert';

interface Test { id: number; question: string; expected_answer: string; created_at: string; }
interface Result { id: number; test_id: number; response: string; score: number; created_at: string; }

export default function EvaluationPage({ params }: any) {
  const id = params.id;
  const [tests, setTests] = useState<Test[]>([]);
  const [results, setResults] = useState<Result[]>([]);
  const [question, setQuestion] = useState('');
  const [expected, setExpected] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>('');
  const [running, setRunning] = useState(false);

  const fetchData = () => {
    setLoading(true);
    Promise.all([
      fetch(`/api/servers/${id}/eval/tests`, { credentials: 'include' }).then(r => r.ok ? r.json() : []),
      fetch(`/api/servers/${id}/eval/results`, { credentials: 'include' }).then(r => r.ok ? r.json() : [])
    ])
      .then(([t, r]) => { setTests(t); setResults(r); })
      .finally(() => setLoading(false));
  };

  useEffect(fetchData, [id]);

  const addTest = () => {
    setError('');
    fetch(`/api/servers/${id}/eval/tests`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ question, expected_answer: expected })
    })
      .then(r => { if (!r.ok) throw new Error('failed'); return r.json(); })
      .then(() => { setQuestion(''); setExpected(''); fetchData(); })
      .catch(() => setError('Failed to add test'));
  };

  const run = () => {
    setRunning(true);
    setError('');
    fetch(`/api/servers/${id}/eval/run`, { method: 'POST', credentials: 'include' })
      .then(r => { if (!r.ok) throw new Error('failed'); })
      .then(fetchData)
      .catch(() => setError('Failed to run tests'))
      .finally(() => setRunning(false));
  };

  return (
    <div className="space-y-4">
      {error && <Alert message={error} />}
      {loading ? (
        <Spinner />
      ) : (
        <>
          <h2 className="text-lg font-semibold">Tests</h2>
          <ul className="space-y-2 mb-4">
            {tests.map(t => (
              <li key={t.id} className="border p-2 rounded">
                <div className="text-sm text-gray-400">{t.created_at}</div>
                <div className="font-medium">Q: {t.question}</div>
                <div className="text-sm">Expected: {t.expected_answer}</div>
              </li>
            ))}
          </ul>
          <div className="space-y-2">
            <input value={question} onChange={e => setQuestion(e.target.value)} placeholder="question" className="w-full px-2 py-1 bg-gray-900 rounded" />
            <input value={expected} onChange={e => setExpected(e.target.value)} placeholder="expected answer" className="w-full px-2 py-1 bg-gray-900 rounded" />
            <Button onClick={addTest}>Add Test</Button>
          </div>
          <Button onClick={run} disabled={running} className="mt-4">
            {running ? 'Running...' : 'Run Tests'}
          </Button>
          <h2 className="text-lg font-semibold mt-6">Results</h2>
          <ul className="space-y-2">
            {results.map(r => (
              <li key={r.id} className="border p-2 rounded">
                <div className="text-sm text-gray-400">{r.created_at}</div>
                <div className="font-medium">Test {r.test_id} Score: {r.score.toFixed(2)}</div>
                <pre className="whitespace-pre-wrap text-sm mt-1">{r.response}</pre>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
