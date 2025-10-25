import Link from 'next/link';

export default function Footer() {
  return (
    <footer className="py-8 text-center text-sm text-gray-500 border-t mt-16">
      <p className="mb-2">© 2025 MCP Host</p>
      <nav className="flex justify-center gap-4">
        <Link href="/docs" className="hover:underline">Docs</Link>
        <Link href="/blog" className="hover:underline">Blog</Link>
        <Link href="/marketplace" className="hover:underline">Marketplace</Link>
      </nav>
    </footer>
  );
}
