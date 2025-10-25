import useSWR from 'swr';

const fetcher = (url: string) => fetch(url, { credentials: 'include' })
  .then(res => {
    if (!res.ok) throw new Error(res.statusText);
    return res.json();
  });

export function useApi<T>(url: string) {
  const { data, error, isLoading, mutate } = useSWR<T>(url, fetcher);
  return { data, error, isLoading, mutate };
}
