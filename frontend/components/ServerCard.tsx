'use client';
import Spinner from './Spinner';
import Link from 'next/link';
import { Server } from '../lib/store';

/* musikconnect:
   purpose: Display and control a single MCP server with actions
   inputs: server data, callbacks for start/stop/delete/redeploy and toggles
   outputs: list item containing controls and optional logs/metrics
   status: stable
   depends_on: Spinner, Next.js Link
   related_docs: ../../design-vision.md
*/

interface Props {
  server: Server;
  actionId: number | null;
  start: (id: number) => Promise<void>;
  stop: (id: number) => Promise<void>;
  del: (id: number) => Promise<void>;
  redeploy: (id: number) => Promise<void>;
  viewLogs: (id: number) => Promise<void>;
  viewMetrics: (id: number) => Promise<void>;
  closeLogs: () => void;
  logs: boolean;
  metrics: boolean;
}

export default function ServerCard({
  server,
  actionId,
  start,
  stop,
  del,
  redeploy,
  viewLogs,
  viewMetrics,
  closeLogs,
  logs,
  metrics,
}: Props) {
  return (
    <li className="border rounded p-4 space-y-2">
      <div className="flex justify-between items-center">
        <div>
          <span className="font-semibold">{server.name}</span>{' '}
          <span className="text-sm text-gray-500">({server.server_type})</span>
          {server.use_gpu && (
            <span className="ml-2 text-xs text-purple-600">GPU</span>
          )}
        </div>
        <span className="capitalize text-sm">{server.status}</span>
      </div>
      <div className="space-x-2 flex flex-wrap">
        {server.status === 'stopped' && (
          <button
            className="px-2 py-1 bg-green-600 text-white flex items-center justify-center rounded"
            onClick={() => start(server.id)}
            disabled={actionId === server.id}
          >
            {actionId === server.id ? <Spinner /> : 'Start'}
          </button>
        )}
        {server.status === 'running' && (
          <button
            className="px-2 py-1 bg-yellow-600 text-white flex items-center justify-center rounded"
            onClick={() => stop(server.id)}
            disabled={actionId === server.id}
          >
            {actionId === server.id ? <Spinner /> : 'Stop'}
          </button>
        )}
        <button
          className="px-2 py-1 bg-red-600 text-white flex items-center justify-center rounded"
          onClick={() => del(server.id)}
          disabled={actionId === server.id}
        >
          {actionId === server.id ? <Spinner /> : 'Delete'}
        </button>
        <button
          className="px-2 py-1 bg-blue-600 text-white flex items-center justify-center rounded"
          onClick={() => redeploy(server.id)}
          disabled={actionId === server.id}
        >
          {actionId === server.id ? <Spinner /> : 'Redeploy'}
        </button>
        <button className="px-2 py-1 bg-gray-600 text-white rounded" onClick={() => viewLogs(server.id)}>
          Logs
        </button>
        <button className="px-2 py-1 bg-gray-600 text-white rounded" onClick={() => viewMetrics(server.id)}>
          Metrics
        </button>
        <Link href={`/servers/${server.id}/services`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Services
        </Link>
        <Link href={`/servers/${server.id}/domains`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Domains
        </Link>
        <Link href={`/servers/${server.id}/files`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Files
        </Link>
        <Link href={`/servers/${server.id}/invoke`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Invoke
        </Link>
        <Link href={`/servers/${server.id}/manifest`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Manifest
        </Link>
        <Link href={`/servers/${server.id}/capabilities`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Capabilities
        </Link>
        <Link href={`/servers/${server.id}/invocations`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Invocations
        </Link>
        <Link href={`/servers/${server.id}/eval`} className="px-2 py-1 bg-gray-600 text-white rounded">
          Evaluation
        </Link>
        {logs && (
          <button className="px-2 py-1 bg-gray-600 text-white rounded" onClick={closeLogs}>
            Close
          </button>
        )}
      </div>
      {logs && (
        <pre className="mt-2 whitespace-pre-wrap bg-black text-green-300 p-2 rounded overflow-auto max-h-60 text-sm">
          {/* logs inserted by parent */}
        </pre>
      )}
      {metrics && (
        <div className="mt-2 bg-gray-900 p-2 rounded">
          {/* metrics chart inserted by parent */}
        </div>
      )}
    </li>
  );
}
