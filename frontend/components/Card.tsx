import { ReactNode } from 'react';
import clsx from 'clsx';

/* musikconnect:
   purpose: Generic card container for lists and feature highlights
   inputs: children nodes, optional className
   outputs: styled div wrapper
   status: stable
   depends_on: clsx
   related_docs: ../../design-vision.md
*/

export default function Card({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div className={clsx('rounded border border-gray-700 p-4 shadow-sm bg-gray-800', className)}>
      {children}
    </div>
  );
}
