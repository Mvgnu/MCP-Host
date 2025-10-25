'use client';
import { useState } from 'react';
import Spinner from '../../../../components/Spinner';
import Alert from '../../../../components/Alert';
import { useApi } from '../../../../lib/api';

export default function ServicesPage({ params }: any) {
  const id = params.id;
  const { data: services, error: fetchError, isLoading, mutate } = useApi<any[]>(`/api/servers/${id}/services`);
  const [serviceType, setServiceType] = useState('Redis');
  const [config, setConfig] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [editConfig, setEditConfig] = useState('');

  const fetchServices = () => mutate();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError(null);
    let cfg: any = {};
    if (config.trim()) {
      try {
        cfg = JSON.parse(config);
      } catch (e) {
        setLoading(false);
        setError('Invalid JSON');
        return;
      }
    }
    const res = await fetch(`/api/servers/${id}/services`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ service_type: serviceType, config: cfg })
    });
    setLoading(false);
    if (res.ok) {
      setConfig('');
      fetchServices();
    } else {
      setError(await res.text());
    }
  };

  const startEdit = (svc: any) => {
    setEditing(svc.id);
    setEditConfig(JSON.stringify(svc.config ?? {}, null, 2));
  };

  const cancelEdit = () => {
    setEditing(null);
    setEditConfig('');
  };

  const saveEdit = async (svcId: number) => {
    setLoading(true);
    setError(null);
    let cfg: any = {};
    if (editConfig.trim()) {
      try {
        cfg = JSON.parse(editConfig);
      } catch (e) {
        setLoading(false);
        setError('Invalid JSON');
        return;
      }
    }
    const res = await fetch(`/api/servers/${id}/services/${svcId}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ config: cfg })
    });
    setLoading(false);
    if (res.ok) {
      cancelEdit();
      fetchServices();
    } else {
      setError(await res.text());
    }
  };

  const remove = async (svcId: number) => {
    setLoading(true);
    setError(null);
    const res = await fetch(`/api/servers/${id}/services/${svcId}`, {
      method: 'DELETE',
      credentials: 'include'
    });
    setLoading(false);
    if (res.ok) {
      fetchServices();
    } else {
      setError(await res.text());
    }
  };

  return (
    <div className="p-4 max-w-md mx-auto space-y-4">
      <h1 className="text-xl font-semibold">Service Integrations</h1>
      {(error || fetchError) && <Alert message={(error || fetchError.message) as string} />}
      {isLoading && <Spinner />}
      <ul className="space-y-2">
        {services?.map(s => (
          <li key={s.id} className="border p-2 rounded">
            <div className="flex justify-between items-center">
              <span className="font-semibold">{s.service_type}</span>
              <div className="space-x-2 text-sm">
                <button className="px-2 py-1 bg-gray-600 text-white rounded" onClick={() => startEdit(s)}>Edit</button>
                <button className="px-2 py-1 bg-red-600 text-white rounded" onClick={() => remove(s.id)}>Delete</button>
              </div>
            </div>
            {editing === s.id && (
              <div className="mt-2 space-y-2">
                <textarea value={editConfig} onChange={e => setEditConfig(e.target.value)} className="border p-2 w-full h-24" />
                <div className="flex space-x-2">
                  <button className="flex-1 bg-blue-600 text-white p-1" onClick={() => saveEdit(s.id)} disabled={loading}>{loading ? <Spinner /> : 'Save'}</button>
                  <button className="flex-1 bg-gray-500 text-white p-1" onClick={cancelEdit}>Cancel</button>
                </div>
              </div>
            )}
          </li>
        ))}
      </ul>
      <form onSubmit={handleSubmit} className="space-y-2">
        <select value={serviceType} onChange={e => setServiceType(e.target.value)} className="border p-2 w-full">
          <option value="Redis">Redis</option>
          <option value="S3">S3</option>
        </select>
        <textarea value={config} onChange={e => setConfig(e.target.value)} placeholder="Service config JSON" className="border p-2 w-full h-24" />
        <button type="submit" disabled={loading} className="bg-blue-600 text-white p-2 w-full flex justify-center">
          {loading ? <Spinner /> : 'Add Service'}
        </button>
      </form>
    </div>
  );
}
