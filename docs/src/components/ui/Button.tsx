import Link from "next/link";
import type { ComponentProps } from "react";

type Variant = "primary" | "secondary" | "ghost";

type Props = {
  href: string;
  external?: boolean;
  variant?: Variant;
  children: React.ReactNode;
  className?: string;
};

const STYLES: Record<Variant, string> = {
  primary: "bg-cyan-600 hover:bg-cyan-700 text-white",
  secondary:
    "border border-black/10 dark:border-white/20 hover:bg-black/5 dark:hover:bg-white/5",
  ghost: "hover:text-cyan-600 dark:hover:text-cyan-400",
};

const BASE =
  "inline-flex items-center justify-center px-6 py-3 rounded-lg font-medium transition";

export function Button({
  href,
  external,
  variant = "primary",
  className = "",
  children,
}: Props) {
  const cls = `${BASE} ${STYLES[variant]} ${className}`;
  if (external) {
    const extraProps: ComponentProps<"a"> = {
      target: "_blank",
      rel: "noreferrer",
    };
    return (
      <a href={href} className={cls} {...extraProps}>
        {children}
      </a>
    );
  }
  return (
    <Link href={href} className={cls}>
      {children}
    </Link>
  );
}
