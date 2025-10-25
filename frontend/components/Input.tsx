'use client';
import clsx from 'clsx';

/* musikconnect:
   purpose: Reusable text input with optional label and focus styles
   inputs: React.InputHTMLAttributes extended with 'label'
   outputs: styled input element wrapped in a label
   status: stable
   depends_on: clsx
   related_docs: ../../design-vision.md
*/

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
}

export default function Input({ label, className, ...props }: InputProps) {
  return (
    <label className="flex flex-col gap-1">
      {label && <span className="text-sm font-medium text-gray-700">{label}</span>}
      <input
        className={clsx(
          'border rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-indigo-500',
          className,
        )}
        {...props}
      />
    </label>
  );
}
