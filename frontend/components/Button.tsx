'use client';

/* musikconnect:
   purpose: Reusable button or link styled with Tailwind variants
   inputs: href optional, onClick callback, variant, disabled flag, className
   outputs: button or Link element
   status: stable
   depends_on: clsx, next/link
   related_docs: ../../design-vision.md
*/
import clsx from 'clsx';
import Link from 'next/link';

interface ButtonProps {
  href?: string;
  onClick?: () => void;
  children: React.ReactNode;
  variant?: 'primary' | 'secondary';
  disabled?: boolean;
  className?: string;
}

export default function Button({ href, onClick, children, variant = 'primary', disabled = false, className }: ButtonProps) {
  const base = 'inline-block px-5 py-2 rounded font-medium transition-colors';
  const styles = {
    primary: 'bg-blue-600 hover:bg-blue-700 text-white',
    secondary: 'bg-gray-200 hover:bg-gray-300 text-gray-900',
  };
  if (href) {
    return (
      <Link href={href} className={clsx(base, styles[variant], className)}>
        {children}
      </Link>
    );
  }
  return (
    <button onClick={onClick} disabled={disabled} className={clsx(base, styles[variant], className, disabled && 'opacity-50 cursor-not-allowed')}> 
      {children}
    </button>
  );
}
