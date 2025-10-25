import Section from '../../components/Section';

const posts = [
  {
    id: 1,
    title: 'Introducing MCP Host',
    excerpt: 'A new way to manage Model Context Providers with ease.'
  },
  {
    id: 2,
    title: 'Custom Images Explained',
    excerpt: 'How to bring your own MCP image for maximum flexibility.'
  }
];

export default function BlogPage() {
  return (
    <div className="space-y-8">
      {posts.map(p => (
        <Section key={p.id} title={p.title}>
          <p>{p.excerpt}</p>
        </Section>
      ))}
    </div>
  );
}
