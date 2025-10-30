import AcceptInvitation from '../../../../components/onboarding/AcceptInvitation';

// key: onboarding-route -> invitation-landing

type InvitePageProps = {
  params: { token: string };
};

export const metadata = {
  title: 'Accept invitation',
  description:
    'Sign in or create an account to accept your MCP organization invitation.',
};

export default function InvitePage({ params }: InvitePageProps) {
  return (
    <main className="mx-auto flex min-h-screen w-full items-start justify-center bg-slate-50 px-4 py-12">
      <AcceptInvitation token={params.token} />
    </main>
  );
}
