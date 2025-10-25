'use client';
import Card from '../../components/Card';
import { useApi } from '../../lib/api';
import Button from '../../components/Button';

interface Item {
  server_type: string;
  description: string;
}

export default function MarketplacePage() {
  const { data, isLoading } = useApi<Item[]>('/api/marketplace');

  return (
    <div className="grid md:grid-cols-2 gap-4 mt-6">
      {isLoading && <div>Loading...</div>}
      {data?.map((item) => (
        <Card key={item.server_type}>
          <h3 className="font-semibold text-lg">{item.server_type}</h3>
          <p className="text-sm mt-1 mb-2">{item.description}</p>
          <Button href={`/servers/new?type=${encodeURIComponent(item.server_type)}`}>Deploy</Button>
        </Card>
      ))}
    </div>
  );
}
