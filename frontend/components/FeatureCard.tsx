import Image from 'next/image';
import Link from 'next/link';

interface FeatureCardProps {
  href: string;
  title: string;
  description: string;
  icon: string;
}

export default function FeatureCard({ href, title, description, icon }: FeatureCardProps) {
  return (
    <Link href={href} className="flex flex-col items-center gap-3 p-4 border rounded hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors">
      <Image src={icon} alt="" width={48} height={48} />
      <h3 className="font-semibold text-lg">{title}</h3>
      <p className="text-sm text-center text-gray-600 dark:text-gray-400">{description}</p>
    </Link>
  );
}
