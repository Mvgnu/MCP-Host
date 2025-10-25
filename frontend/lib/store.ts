import { create } from 'zustand';

export interface Server {
  id: number;
  name: string;
  server_type: string;
  status: string;
  use_gpu: boolean;
}

interface ServerState {
  servers: Server[];
  loading: boolean;
  error: string | null;
  fetchServers: () => Promise<void>;
  updateStatus: (id: number, status: string) => void;
}

export const useServerStore = create<ServerState>((set) => ({
  servers: [],
  loading: false,
  error: null,
  fetchServers: async () => {
    set({ loading: true, error: null });
    try {
      const res = await fetch('/api/servers', { credentials: 'include' });
      const data: Server[] = await res.json();
      set({ servers: data, loading: false });
    } catch {
      set({ error: 'Failed to load servers', loading: false });
    }
  },
  updateStatus: (id, status) =>
    set((state) => ({
      servers: state.servers.map((s) =>
        s.id === id ? { ...s, status } : s
      ),
    })),
}));
