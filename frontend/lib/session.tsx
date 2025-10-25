'use client';
import { createContext, useContext, useEffect, useState } from 'react';

export interface User {
  id: number;
  email: string;
}

const SessionContext = createContext<User | null>(null);

export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<User | null>(null);

  useEffect(() => {
    fetch('/api/me')
      .then(res => (res.ok ? res.json() : null))
      .then(data => {
        if (data) setUser(data);
      });
  }, []);

  return <SessionContext.Provider value={user}>{children}</SessionContext.Provider>;
}

export function useSession() {
  return useContext(SessionContext);
}
