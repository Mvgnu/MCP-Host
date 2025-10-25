import { act } from '@testing-library/react';
import { useServerStore } from './store';

// Zustand persists across tests, reset
beforeEach(() => {
  const { servers } = useServerStore.getState();
  useServerStore.setState({ servers: [] });
});

test('updateStatus updates server status', () => {
  useServerStore.setState({ servers: [{ id: 1, name: 's', server_type: 't', status: 'starting', use_gpu: false }] });
  act(() => {
    useServerStore.getState().updateStatus(1, 'running');
  });
  expect(useServerStore.getState().servers[0].status).toBe('running');
});
