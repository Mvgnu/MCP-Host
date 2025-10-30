'use client';

// key: onboarding-ui -> invitation-acceptance

import { ChangeEvent, FormEvent, useMemo, useState } from 'react';
import Alert from '../Alert';
import Button from '../Button';
import Card from '../Card';
import Input from '../Input';
import Spinner from '../Spinner';
import { acceptInvitation, Invitation } from '../../lib/organizations';
import { loginUser, registerUser } from '../../lib/auth';

const PASSWORD_MIN = 8;

type Mode = 'register' | 'login';

type FormState = {
  email: string;
  password: string;
  confirm: string;
};

type Services = {
  registerUser: typeof registerUser;
  loginUser: typeof loginUser;
  acceptInvitation: typeof acceptInvitation;
};

const DEFAULT_SERVICES: Services = {
  registerUser,
  loginUser,
  acceptInvitation,
};

export interface AcceptInvitationProps {
  token: string;
  services?: Partial<Services>;
}

export default function AcceptInvitation({
  token,
  services: overrides,
}: AcceptInvitationProps) {
  const services: Services = useMemo(
    () => ({ ...DEFAULT_SERVICES, ...overrides }),
    [overrides],
  );
  const [mode, setMode] = useState<Mode>('register');
  const [form, setForm] = useState<FormState>({
    email: '',
    password: '',
    confirm: '',
  });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<Invitation | null>(null);

  const handleInputChange = (field: keyof FormState) =>
    ({ target }: ChangeEvent<HTMLInputElement>) => {
      const { value } = target;
      setForm((prev) => ({ ...prev, [field]: value }));
      setError(null);
    };

  const resetState = () => {
    setForm({ email: '', password: '', confirm: '' });
    setSubmitting(false);
    setError(null);
  };

  const handleModeToggle = (nextMode: Mode) => {
    if (mode === nextMode) {
      return;
    }
    setMode(nextMode);
    resetState();
  };

  const validateForm = (): string | null => {
    if (!form.email.trim()) {
      return 'Email is required.';
    }
    if (!form.password || form.password.length < PASSWORD_MIN) {
      return `Password must be at least ${PASSWORD_MIN} characters.`;
    }
    if (mode === 'register' && form.password !== form.confirm) {
      return 'Passwords do not match.';
    }
    return null;
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (submitting) {
      return;
    }
    const validationError = validateForm();
    if (validationError) {
      setError(validationError);
      return;
    }

    setSubmitting(true);
    setError(null);

    try {
      if (mode === 'register') {
        await services.registerUser({
          email: form.email.trim(),
          password: form.password,
        });
      }

      await services.loginUser({
        email: form.email.trim(),
        password: form.password,
      });

      const invitation = await services.acceptInvitation(token);
      setSuccess(invitation);
    } catch (err) {
      const message =
        err instanceof Error
          ? err.message || 'Failed to accept invitation.'
          : 'Failed to accept invitation.';
      setError(message);
      setSubmitting(false);
      return;
    }

    setSubmitting(false);
  };

  const renderForm = () => (
    <form className="flex flex-col gap-4" onSubmit={handleSubmit}>
      <Input
        id="invite-email"
        label="Work Email"
        type="email"
        value={form.email}
        onChange={handleInputChange('email')}
        required
      />
      <Input
        id="invite-password"
        label="Password"
        type="password"
        value={form.password}
        onChange={handleInputChange('password')}
        minLength={PASSWORD_MIN}
        required
      />
      {mode === 'register' ? (
        <Input
          id="invite-confirm"
          label="Confirm Password"
          type="password"
          value={form.confirm}
          onChange={handleInputChange('confirm')}
          minLength={PASSWORD_MIN}
          required
        />
      ) : null}
      <Button type="submit" disabled={submitting}>
        {submitting ? (
          <span className="flex items-center justify-center gap-2">
            <Spinner size="sm" />
            Processing invitation...
          </span>
        ) : mode === 'register' ? (
          'Register and Accept'
        ) : (
          'Sign In and Accept'
        )}
      </Button>
    </form>
  );

  return (
    <Card className="mx-auto w-full max-w-lg">
      <div className="mb-4 flex justify-between">
        <h1 className="text-2xl font-semibold">Join your team</h1>
        <div className="flex gap-2">
          <Button
            type="button"
            variant={mode === 'register' ? 'primary' : 'secondary'}
            disabled={submitting}
            onClick={() => handleModeToggle('register')}
          >
            New user
          </Button>
          <Button
            type="button"
            variant={mode === 'login' ? 'primary' : 'secondary'}
            disabled={submitting}
            onClick={() => handleModeToggle('login')}
          >
            Existing user
          </Button>
        </div>
      </div>
      <p className="mb-6 text-sm text-slate-600">
        Use the email address that received the invitation. We&apos;ll create your
        account if you&apos;re new here, or sign you in before accepting the invite.
      </p>
      {error ? <Alert tone="danger">{error}</Alert> : null}
      {success ? (
        <div className="flex flex-col gap-4">
          <Alert tone="success">
            Invitation accepted! You now have access to organization #{' '}
            {success.organization_id}. You can continue to the console to get
            started.
          </Alert>
        </div>
      ) : (
        renderForm()
      )}
    </Card>
  );
}
