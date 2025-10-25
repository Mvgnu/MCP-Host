'use client';
import { useState } from 'react';
import Spinner from '../../components/Spinner';
import Alert from '../../components/Alert';
import Input from '../../components/Input';
import { useRouter } from 'next/navigation';

export default function Login() {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const router = useRouter();
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    const res = await fetch('/api/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, password }),
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
      <h1 className="text-xl font-semibold mb-2 text-center">Login</h1>
      <Input
        type="email"
        value={email}
        onChange={e => setEmail(e.target.value)}
        label="Email"
        required
      />
      <Input
        type="password"
        value={password}
        onChange={e => setPassword(e.target.value)}
        label="Password"
        required
      />
      {error && <Alert message={error} />}
      <button className="bg-blue-600 text-white p-2 flex items-center justify-center" type="submit" disabled={loading}>
        {loading ? <Spinner /> : 'Login'}
      </button>
    </form>
  );
}
