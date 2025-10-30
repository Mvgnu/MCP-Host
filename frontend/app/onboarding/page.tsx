import SelfServiceOnboarding from '../../components/onboarding/SelfServiceOnboarding';

// key: onboarding-route -> self-service-entrypoint

export const metadata = {
  title: 'Self-service onboarding',
  description:
    'Create an MCP SaaS organization, pick a plan, and invite teammates without waiting on support.',
};

export default function OnboardingPage() {
  return (
    <main className="mx-auto flex min-h-screen w-full items-start justify-center bg-slate-50 px-4 py-12">
      <SelfServiceOnboarding />
    </main>
  );
}
