'use client';
import { useState } from 'react';
import Spinner from '../../../../components/Spinner';
import Alert from '../../../../components/Alert';

export default function InvokePage({ params }: any) {
  const id = params.id;
  const [payload, setPayload] = useState('{\n  "input": "hello"\n}');
  const [response, setResponse] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const invoke = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError(null);
    const res = await fetch(`/api/servers/${id}/invoke`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: payload,
    });
    setLoading(false);
    if (res.ok) {
      setResponse(await res.text());
    } else {
      setError(await res.text());
    }
  };

  return (
    <div className="p-4 max-w-xl mx-auto space-y-4">
      <h1 className="text-xl font-semibold">Invoke Server</h1>
      {error && <Alert message={error} />}
      <form onSubmit={invoke} className="space-y-2">
        <textarea
          className="border w-full p-2 h-32"
          value={payload}
          onChange={e => setPayload(e.target.value)}
        />
        <button
          type="submit"
          disabled={loading}
          className="bg-blue-600 text-white p-2 w-full flex justify-center"
        >
          {loading ? <Spinner /> : 'Send'}
        </button>
      </form>
      {response && (
        <pre className="bg-black text-green-300 p-2 rounded whitespace-pre-wrap">
          {response}
        </pre>
      )}
    </div>
  );
}
