import Section from '../../components/Section';
import Button from '../../components/Button';

export default function DocsPage() {
  return (
    <div className="space-y-8">
      <Section title="Getting Started">
        <p>
          MCP Host lets you deploy Model Context Providers on demand. Start by
          creating an account and launching your first server.
        </p>
        <Button href="/servers/new">Create a server</Button>
      </Section>
      <Section title="Bring Your Own MCP">
        <p>
          Prefer a custom image? Select the <em>Custom (BYO image)</em> option
          when creating a server and provide your Docker image name.
        </p>
        <p className="mt-2">
          You can also pass configuration values as environment variables by
          entering <code>KEY=value</code> pairs in the form. These are injected
          into the container with a <code>CFG_</code> prefix.
        </p>
        <ul className="list-disc pl-6 mt-2 space-y-1">
          <li>
            Example: <code>API_TOKEN=my-secret</code> becomes{' '}
            <code>CFG_API_TOKEN</code> inside your container.
          </li>
          <li>
            Use any image from Docker Hub or a private registry, e.g.
            <code>ghcr.io/you/custom-mcp:latest</code>.
          </li>
          <li>
            If your image expects a custom entrypoint or command, bake it into
            the image itself so MCP Host can run it with a simple{' '}
            <code>docker run</code>.
          </li>
          <li>
            Alternatively provide a git repository URL and MCP Host will clone
            and build the Docker image automatically.
          </li>
          <li>
            Specify a <code>branch</code> to build from if you don't want the
            default <code>main</code> branch.
          </li>
          <li>
            Trigger redeploys by calling the <code>/api/servers/&lt;id&gt;/webhook?secret=...</code>
            endpoint from your CI or Git provider. The secret is returned when the server is created.
          </li>
          <li>
            GitHub users can configure a push webhook pointing to
            <code>/api/servers/&lt;id&gt;/github</code> with the same secret to
            verify the payload.
          </li>
          <li>
            The server status will show <em>cloning</em> and <em>building</em>
          while the image is prepared, and progress logs appear on the Logs
          tab.
        </li>
        <li>
          Builds run in a temporary directory with <code>--pull</code> and
          <code>--no-cache</code> flags for added security, so each build uses
          fresh base images and no leftover layers.
        </li>
          <li>
            MCP Host parses your Dockerfile for <code>EXPOSE</code> instructions
            and warns if port <code>8080</code> is not exposed, which the reverse
            proxy expects.
          </li>
          <li>
            If no Dockerfile is present, MCP Host detects Node, Python, or Rust
            projects and generates one automatically before building.
          </li>
          <li>
            Set a <code>REGISTRY</code> environment variable and built images
            will be pushed after the build completes.
          </li>
          <li>
            Enable the <strong>GPU</strong> option when creating a server to run
            the container with Nvidia GPUs if available.
          </li>
          <li>
            MCP Host automatically restarts containers if they crash so your
            deployments stay online.
          </li>
          <li>
            Test your deployment by sending JSON to
            <code>/api/servers/&lt;id&gt;/invoke</code> or using the Invoke page
            from the dashboard.
          </li>
          <li>
            After the container starts, MCP Host fetches
            <code>/.well-known/mcp.json</code> and stores the manifest so agents
            can plug in automatically.
          </li>
          <li>
            Capabilities listed in the manifest are saved and available from
            <code>/api/servers/&lt;id&gt;/capabilities</code> for easy agent
            configuration.
          </li>
          <li>
            Fetch <code>/api/servers/&lt;id&gt;/client-config</code> to receive
            a ready-to-use configuration snippet with the invoke URL,
            API key, and stored manifest.
          </li>
        </ul>
      </Section>
      <Section title="Streaming Metrics">
        <p>
          Servers publish real-time usage metrics via Server-Sent Events. Our
          dashboard visualizes these with interactive charts so you can monitor
          activity live.
        </p>
      </Section>
      <Section title="Prebuilt Service Integrations">
        <p>
          Attach managed services like Redis or S3 to your MCP servers with a few clicks.
          Provide any connection details as JSON when adding a service and MCP Host
          injects the relevant environment variables (<code>REDIS_URL</code>, <code>S3_BUCKET</code>, etc.)
          when the container starts.
        </p>
        <p className="mt-2">
          You can edit or remove integrations later from the Services page.
        </p>
      </Section>
      <Section title="Ingestion Pipelines">
        <p>
          Create recurring jobs that fetch data from an external URL and ingest
          it into one of your managed vector databases. Use the
          <code>/api/ingestion-jobs</code> endpoints to manage these jobs.
        </p>
      </Section>
      <Section title="Workflows">
        <p>
          Combine multiple MCP servers into a pipeline. Use <code>/api/workflows</code> to create and invoke them.
        </p>
      </Section>
      <Section title="Resource Limits">
        <p>
          Constrain server resource usage by including <code>cpu_limit</code> (number of cores)
          and <code>memory_limit</code> (MB) in the server configuration JSON. Both Docker and
          Kubernetes runtimes respect these limits when launching containers.
        </p>
      </Section>
    </div>
  );
}
