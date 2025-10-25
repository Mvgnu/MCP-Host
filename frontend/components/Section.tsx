interface SectionProps {
  title: string;
  children: React.ReactNode;
}

export default function Section({ title, children }: SectionProps) {
  return (
    <section className="my-12 max-w-3xl mx-auto px-4">
      <h2 className="text-2xl font-semibold mb-4 border-b pb-2">{title}</h2>
      <div className="text-gray-700 dark:text-gray-200 leading-relaxed space-y-4">
        {children}
      </div>
    </section>
  );
}
