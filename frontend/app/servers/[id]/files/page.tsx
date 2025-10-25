'use client';
import { useState, useEffect } from 'react';
import Spinner from '../../../../components/Spinner';
import Alert from '../../../../components/Alert';

export default function FilesPage({ params }: any) {
  const id = params.id;
  const [files, setFiles] = useState<any[]>([]);
  const [file, setFile] = useState<File | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchFiles = () => {
    fetch(`/api/servers/${id}/files`, { credentials: 'include' })
      .then(res => res.json())
      .then(setFiles)
      .catch(() => setError('Failed to load files'));
  };

  useEffect(() => {
    fetchFiles();
  }, []);

  const upload = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!file) return;
    setLoading(true);
    setError(null);
    const form = new FormData();
    form.append('file', file);
    const res = await fetch(`/api/servers/${id}/files`, {
      method: 'POST',
      credentials: 'include',
      body: form,
    });
    setLoading(false);
    if (res.ok) {
      setFile(null);
      fetchFiles();
    } else {
      setError(await res.text());
    }
  };

  const remove = async (fid: number) => {
    setLoading(true);
    setError(null);
    const res = await fetch(`/api/servers/${id}/files/${fid}`, {
      method: 'DELETE',
      credentials: 'include',
    });
    setLoading(false);
    if (res.ok) {
      fetchFiles();
    } else {
      setError(await res.text());
    }
  };

  return (
    <div className="p-4 max-w-md mx-auto space-y-4">
      <h1 className="text-xl font-semibold">Files</h1>
      {error && <Alert message={error} />}
      <ul className="space-y-2">
        {files.map(f => (
          <li key={f.id} className="border p-2 rounded flex justify-between items-center">
            <a href={`/api/servers/${id}/files/${f.id}`} className="underline">
              {f.name}
            </a>
            <button className="px-2 py-1 bg-red-600 text-white rounded" onClick={() => remove(f.id)}>
              Delete
            </button>
          </li>
        ))}
      </ul>
      <form onSubmit={upload} className="space-y-2">
        <input type="file" onChange={e => setFile(e.target.files?.[0] || null)} className="border p-2 w-full" />
        <button type="submit" disabled={loading} className="bg-blue-600 text-white p-2 w-full flex justify-center">
          {loading ? <Spinner /> : 'Upload'}
        </button>
      </form>
    </div>
  );
}
