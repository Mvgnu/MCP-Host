'use client';
import { useState } from 'react';
import { useApi } from '../../lib/api';
import Card from '../../components/Card';
import Button from '../../components/Button';
import Alert from '../../components/Alert';

interface Workflow { id:number; name:string; created_at:string; }
interface Server { id:number; name:string; }

export default function WorkflowsPage() {
  const { data: workflows, isLoading, mutate } = useApi<Workflow[]>('/api/workflows');
  const { data: servers } = useApi<Server[]>('/api/servers');
  const [name, setName] = useState('');
  const [selected, setSelected] = useState<number[]>([]);
  const [input, setInput] = useState('{}');
  const [error, setError] = useState('');

  const toggle = (id:number) => {
    setSelected(prev => prev.includes(id) ? prev.filter(x=>x!==id) : [...prev, id]);
  };

  const create = async () => {
    setError('');
    const res = await fetch('/api/workflows', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, steps: selected }),
    });
    if (res.ok) {
      setName('');
      setSelected([]);
      mutate();
    } else {
      setError('Failed to create');
    }
  };

  const del = async (id:number) => {
    await fetch(`/api/workflows/${id}`, { method: 'DELETE', credentials:'include' });
    mutate();
  };

  const invoke = async (id:number) => {
    let parsed: any;
    try { parsed = JSON.parse(input); } catch { setError('Invalid JSON'); return; }
    const res = await fetch(`/api/workflows/${id}/invoke`, {
      method:'POST', credentials:'include', headers:{'Content-Type':'application/json'},
      body: JSON.stringify({ input: parsed })
    });
    if(res.ok){
      const out = await res.json();
      alert(JSON.stringify(out));
    } else {
      setError('Failed to invoke');
    }
  };

  return (
    <div className="space-y-4 mt-6">
      <div className="flex gap-2 items-start flex-wrap">
        <input value={name} onChange={e=>setName(e.target.value)} className="border p-2" placeholder="Workflow name" />
        <div className="flex flex-wrap gap-2">
          {servers?.map(s => (
            <label key={s.id} className="text-sm flex items-center gap-1">
              <input type="checkbox" checked={selected.includes(s.id)} onChange={()=>toggle(s.id)} />
              {s.name}
            </label>
          ))}
        </div>
        <Button onClick={create}>Create</Button>
      </div>
      <textarea value={input} onChange={e=>setInput(e.target.value)} className="border p-2 w-full h-24" placeholder="Invoke input as JSON" />
      {error && <Alert message={error} />}
      {isLoading && <div>Loading...</div>}
      <div className="grid md:grid-cols-2 gap-4">
        {workflows?.map(w => (
          <Card key={w.id} className="space-y-2">
            <div className="font-semibold">{w.name}</div>
            <div className="text-xs text-gray-500">{new Date(w.created_at).toLocaleString()}</div>
            <div className="flex gap-2">
              <Button onClick={()=>invoke(w.id)}>Run</Button>
              <Button onClick={()=>del(w.id)} variant="secondary">Delete</Button>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
