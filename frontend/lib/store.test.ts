import { act } from '@testing-library/react';
import { useServerStore } from './store';

const originalFetch = globalThis.fetch;

// Zustand persists across tests, reset
beforeEach(() => {
  useServerStore.setState({ servers: [], loading: false, error: null });
  globalThis.fetch = originalFetch;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
  jest.restoreAllMocks();
});

test('updateStatus updates server status', () => {
  useServerStore.setState({ servers: [{ id: 1, name: 's', server_type: 't', status: 'starting', use_gpu: false }] });
  act(() => {
    useServerStore.getState().updateStatus(1, 'running');
  });
  expect(useServerStore.getState().servers[0].status).toBe('running');
});

test('fetchServers populates servers on success', async () => {
  const payload = [{ id: 9, name: 'ops', server_type: 'vm', status: 'active', use_gpu: true }];
  const fetchMock = jest.fn().mockResolvedValue({
    json: async () => payload,
  } as unknown as Response);
  globalThis.fetch = fetchMock;

  await act(async () => {
    await useServerStore.getState().fetchServers();
  });

  expect(fetchMock).toHaveBeenCalledWith('/api/servers', { credentials: 'include' });
  expect(useServerStore.getState().servers).toEqual(payload);
  expect(useServerStore.getState().loading).toBe(false);
  expect(useServerStore.getState().error).toBeNull();
});

test('fetchServers records error on failure', async () => {
  const fetchMock = jest.fn().mockRejectedValue(new Error('network error'));
  globalThis.fetch = fetchMock;

  await act(async () => {
    await useServerStore.getState().fetchServers();
  });

  expect(fetchMock).toHaveBeenCalled();
  expect(useServerStore.getState().servers).toEqual([]);
  expect(useServerStore.getState().loading).toBe(false);
  expect(useServerStore.getState().error).toBe('Failed to load servers');
});
