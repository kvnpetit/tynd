type Props = {
  title: string;
  description: string;
  icon?: React.ReactNode;
};

export function FeatureCard({ title, description, icon }: Props) {
  return (
    <div className="space-y-2">
      {icon && (
        <div className="inline-flex w-10 h-10 items-center justify-center rounded-lg bg-cyan-600/10 text-cyan-600 dark:text-cyan-400">
          {icon}
        </div>
      )}
      <h3 className="text-lg font-semibold">{title}</h3>
      <p className="text-sm text-neutral-600 dark:text-neutral-400 leading-relaxed">
        {description}
      </p>
    </div>
  );
}
