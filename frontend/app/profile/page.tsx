'use client';
import { useEffect, useState } from 'react';
import Alert from '../../components/Alert';

interface UserInfo { id: number; email: string; role: string; server_quota: number; }
interface Server { id: number; name: string; }

export default function ProfilePage() {
  const [user, setUser] = useState<UserInfo | null>(null);
  const [servers, setServers] = useState<Server[]>([]);
  const [error, setError] = useState('');

  useEffect(() => {
    fetch('/api/me', { credentials: 'include' })
      .then(r => (r.ok ? r.json() : Promise.reject('failed')))
      .then(setUser)
      .catch(() => setError('Failed to load user'));
    fetch('/api/servers', { credentials: 'include' })
      .then(r => (r.ok ? r.json() : Promise.reject('failed')))
      .then(setServers)
      .catch(() => setError('Failed to load servers'));
  }, []);

  if (error) return <Alert message={error} />;
  if (!user) return <p>Loading...</p>;
  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold">Profile</h1>
      <p>Email: {user.email}</p>
      <p>Role: {user.role}</p>
      <p>
        Servers used: {servers.length} / {user.server_quota}
      </p>
    </div>
  );
}
