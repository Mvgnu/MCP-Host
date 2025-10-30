import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import AcceptInvitation from '../AcceptInvitation';

// key: onboarding-tests -> invitation-acceptance

describe('AcceptInvitation', () => {
  const token = 'abc-token';

  it('registers a new user and accepts the invitation', async () => {
    const registerUser = jest.fn().mockResolvedValue(undefined);
    const loginUser = jest.fn().mockResolvedValue(undefined);
    const acceptInvitation = jest.fn().mockResolvedValue({
      id: 'invite-1',
      organization_id: 73,
      email: 'new@example.com',
      status: 'accepted',
      invited_at: new Date().toISOString(),
      accepted_at: new Date().toISOString(),
      expires_at: new Date().toISOString(),
      token,
    });

    render(
      <AcceptInvitation
        token={token}
        services={{ registerUser, loginUser, acceptInvitation }}
      />,
    );

    fireEvent.change(screen.getByLabelText(/Work Email/i), {
      target: { value: 'new@example.com' },
    });
    fireEvent.change(screen.getByLabelText(/^Password$/i), {
      target: { value: 'supersecret' },
    });
    fireEvent.change(screen.getByLabelText(/Confirm Password/i), {
      target: { value: 'supersecret' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: /Register and Accept/i }),
    );

    await waitFor(() => expect(registerUser).toHaveBeenCalled());
    await waitFor(() => expect(loginUser).toHaveBeenCalled());
    await waitFor(() => expect(acceptInvitation).toHaveBeenCalledWith(token));

    expect(
      await screen.findByText(/Invitation accepted! You now have access/i),
    ).toBeInTheDocument();
  });

  it('surfaces API errors when acceptance fails', async () => {
    const loginUser = jest.fn().mockResolvedValue(undefined);
    const acceptInvitation = jest
      .fn()
      .mockRejectedValue(new Error('Invitation already processed'));

    render(
      <AcceptInvitation
        token={token}
        services={{ loginUser, acceptInvitation }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: /Existing user/i }));

    fireEvent.change(screen.getByLabelText(/Work Email/i), {
      target: { value: 'member@example.com' },
    });
    fireEvent.change(screen.getByLabelText(/^Password$/i), {
      target: { value: 'supersecret' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: /Sign In and Accept/i }),
    );

    await waitFor(() => expect(loginUser).toHaveBeenCalled());
    await waitFor(() => expect(acceptInvitation).toHaveBeenCalledWith(token));

    expect(
      await screen.findByText(/Invitation already processed/i),
    ).toBeInTheDocument();
  });

  it('validates password confirmation before submitting', async () => {
    const registerUser = jest.fn();
    const loginUser = jest.fn();
    const acceptInvitation = jest.fn();

    render(
      <AcceptInvitation
        token={token}
        services={{ registerUser, loginUser, acceptInvitation }}
      />,
    );

    fireEvent.change(screen.getByLabelText(/Work Email/i), {
      target: { value: 'new@example.com' },
    });
    fireEvent.change(screen.getByLabelText(/^Password$/i), {
      target: { value: 'supersecret' },
    });
    fireEvent.change(screen.getByLabelText(/Confirm Password/i), {
      target: { value: 'different' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: /Register and Accept/i }),
    );

    expect(await screen.findByText(/Passwords do not match/i)).toBeInTheDocument();
    expect(registerUser).not.toHaveBeenCalled();
    expect(loginUser).not.toHaveBeenCalled();
    expect(acceptInvitation).not.toHaveBeenCalled();
  });
});
