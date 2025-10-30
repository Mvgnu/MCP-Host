'use client';

// key: ui-alert -> status-banner

import { ReactNode } from 'react';

type LegacyType = 'error' | 'success';
type Tone = 'danger' | 'success' | 'warning' | 'info';

type AlertProps = {
  tone?: Tone;
  /** @deprecated Use `tone` instead. */
  type?: LegacyType;
  /** @deprecated Use children instead. */
  message?: string;
  children?: ReactNode;
};

const toneClasses: Record<Tone, string> = {
  danger: 'bg-red-100 text-red-800 border-red-300',
  success: 'bg-green-100 text-green-800 border-green-300',
  warning: 'bg-yellow-100 text-yellow-800 border-yellow-300',
  info: 'bg-blue-100 text-blue-800 border-blue-300',
};

function normalizeTone(tone: Tone | undefined, type: LegacyType | undefined): Tone {
  if (tone) {
    return tone;
  }
  if (type === 'success') {
    return 'success';
  }
  return 'danger';
}

export default function Alert({ tone, type, message, children }: AlertProps) {
  const resolvedTone = normalizeTone(tone, type);
  const content = children ?? message ?? '';
  return (
    <div className={`border ${toneClasses[resolvedTone]} p-2 rounded`} role="alert">
      {content}
    </div>
  );
}
