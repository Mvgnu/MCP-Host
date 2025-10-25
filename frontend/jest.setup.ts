import '@testing-library/jest-dom';
// Simplistic mock for Next.js Link component
jest.mock('next/link', () => {
  return ({ href, children }: { href: string; children: React.ReactNode }) => {
    return React.createElement('a', { href }, children);
  };
});
