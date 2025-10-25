'use client';
import clsx from 'clsx';

/* musikconnect:
   purpose: Reusable textarea with label and Tailwind focus styles
   inputs: React.TextareaHTMLAttributes extended with 'label'
   outputs: styled textarea element wrapped in a label
   status: stable
   depends_on: clsx
   related_docs: ../../design-vision.md
*/

interface TextareaProps extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  label?: string;
}

export default function Textarea({ label, className, ...props }: TextareaProps) {
  return (
    <label className="flex flex-col gap-1">
      {label && <span className="text-sm font-medium text-gray-700">{label}</span>}
      <textarea
        className={clsx('border rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-indigo-500', className)}
        {...props}
      />
    </label>
  );
}
