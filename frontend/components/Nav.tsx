'use client';
import Link from 'next/link';
import { useSession } from '../lib/session';

export default function Nav() {
  const user = useSession();
  return (
    <nav className="p-4 bg-slate-800 text-white flex gap-4">
      <Link href="/">Home</Link>
      <Link href="/docs">Docs</Link>
      <Link href="/blog">Blog</Link>
      <Link href="/marketplace">Marketplace</Link>
      <Link href="/vector-dbs">Vector DBs</Link>
      <Link href="/ingestion">Ingestion</Link>
      <Link href="/workflows">Workflows</Link>
      <Link href="/evaluations">Evaluations</Link>
      <Link href="/orgs">Orgs</Link>
      <Link href="/servers">Servers</Link>
      <Link href="/servers/new">New Server</Link>
      <Link href="/console/lifecycle">Lifecycle Console</Link>
      {user ? (
        <>
          <Link href="/profile">Profile</Link>
          <form action="/api/logout" method="post">
            <button className="ml-4 underline" type="submit">Logout ({user.email})</button>
          </form>
        </>
      ) : (
        <>
          <Link href="/login">Login</Link>
          <Link href="/register">Register</Link>
        </>
      )}
    </nav>
  );
}
