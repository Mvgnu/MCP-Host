import Hero from '../components/Hero';
import Section from '../components/Section';
import Button from '../components/Button';
import FeatureCard from '../components/FeatureCard';

export default function Home() {
  return (
    <div>
      <Hero />
      <Section title="Features">
        <div className="grid md:grid-cols-3 gap-4">
          <FeatureCard
            href="/marketplace"
            title="Marketplace"
            description="Deploy official MCP images in one click"
            icon="/globe.svg"
          />
          <FeatureCard
            href="/vector-dbs"
            title="Vector DBs"
            description="Create managed embeddings stores"
            icon="/file.svg"
          />
          <FeatureCard
            href="/workflows"
            title="Workflows"
            description="Chain MCPs together for complex pipelines"
            icon="/window.svg"
          />
        </div>
      </Section>
      <Section title="Why MCP Host?">
        <p>
          Deploy scalable Model Context Providers backed by Docker with a few
          clicks. Monitor metrics in real time and customize each server to your
          needs.
        </p>
        <div className="mt-4 flex gap-3">
          <Button href="/docs">Read the docs</Button>
          <Button href="/blog" variant="secondary">
            Visit the blog
          </Button>
        </div>
      </Section>
    </div>
  );
}
