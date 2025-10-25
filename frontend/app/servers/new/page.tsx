'use client';
import { useState, useEffect } from 'react';
import Spinner from '../../../components/Spinner';
import Alert from '../../../components/Alert';
import Input from '../../../components/Input';
import Textarea from '../../../components/Textarea';
import { useRouter } from 'next/navigation';

export default function NewServer() {
  const [name, setName] = useState('');
  const [serverType, setServerType] = useState('PostgreSQL');
  const [customImage, setCustomImage] = useState('');
  const [repoUrl, setRepoUrl] = useState('');
  const [branch, setBranch] = useState('main');
  const [envText, setEnvText] = useState('');
  const [useGpu, setUseGpu] = useState(false);
  const [market, setMarket] = useState<{server_type:string}[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const router = useRouter();

  useEffect(() => {
    fetch('/api/marketplace', { credentials: 'include' })
      .then(r => r.ok ? r.json() : [])
      .then(setMarket)
      .catch(() => {});
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    const body: any = { name, server_type: serverType, use_gpu: useGpu };
    if (serverType === 'Custom') {
      if (customImage) {
        body.config = { image: customImage };
      }
      if (repoUrl) {
        body.config = { ...(body.config || {}), repo_url: repoUrl };
        if (branch) {
          body.config.branch = branch;
        }
      }
    }
    if (envText.trim()) {
      const env: Record<string, string> = {};
      envText.split('\n').forEach(line => {
        const [k, ...rest] = line.split('=');
        if (k && rest.length > 0) {
          env[k.trim()] = rest.join('=').trim();
        }
      });
      body.config = { ...(body.config || {}), ...env };
    }
    const res = await fetch('/api/servers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      credentials: 'include',
    });
    setLoading(false);
    if (res.ok) {
      router.push('/servers');
    } else {
      const text = await res.text();
      setError(text);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-4 max-w-sm mx-auto mt-20 bg-white p-6 rounded shadow">
      <h1 className="text-xl font-semibold mb-2 text-center">New Server</h1>
      <Input
        value={name}
        onChange={e => setName(e.target.value)}
        label="Server name"
        required
      />
      <select value={serverType} onChange={e => setServerType(e.target.value)} className="border p-2">
        {market.map(m => (
          <option key={m.server_type} value={m.server_type}>{m.server_type}</option>
        ))}
        <option value="Custom">Custom (BYO image)</option>
      </select>
      {serverType === 'Custom' && (
        <>
          <Input
            value={customImage}
            onChange={e => setCustomImage(e.target.value)}
            label="Docker image"
          />
          <Input
            value={repoUrl}
            onChange={e => setRepoUrl(e.target.value)}
            label="Git repo URL"
          />
          <Input
            value={branch}
            onChange={e => setBranch(e.target.value)}
            label="Branch (default main)"
          />
        </>
      )}
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={useGpu} onChange={e => setUseGpu(e.target.checked)} />
        Use GPU
      </label>
      <Textarea
        value={envText}
        onChange={e => setEnvText(e.target.value)}
        label="ENV_VAR=value per line"
        className="h-24"
      />
      {error && <Alert message={error} />}
      <button type="submit" className="bg-blue-600 text-white p-2 flex items-center justify-center" disabled={loading}>
        {loading ? <Spinner /> : 'Create'}
      </button>
    </form>
  );
}
