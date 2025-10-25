'use client';

export default function Alert({ message, type = 'error' }: { message: string; type?: 'error' | 'success' }) {
  const color = type === 'error' ? 'bg-red-100 text-red-800 border-red-300' : 'bg-green-100 text-green-800 border-green-300';
  return (
    <div className={`border ${color} p-2 rounded`}>{message}</div>
  );
}
