type Props = {
  eyebrow?: string;
  title: string;
  description?: string;
  align?: "left" | "center";
};

export function SectionTitle({
  eyebrow,
  title,
  description,
  align = "center",
}: Props) {
  const alignCls = align === "center" ? "text-center" : "text-left";
  return (
    <div className={alignCls}>
      {eyebrow && (
        <div className="text-sm font-semibold uppercase tracking-wider text-cyan-600 dark:text-cyan-400">
          {eyebrow}
        </div>
      )}
      <h2 className="mt-2 text-3xl sm:text-4xl font-bold tracking-tight">
        {title}
      </h2>
      {description && (
        <p
          className={`mt-4 text-lg text-neutral-600 dark:text-neutral-400 ${
            align === "center" ? "max-w-2xl mx-auto" : ""
          }`}
        >
          {description}
        </p>
      )}
    </div>
  );
}
