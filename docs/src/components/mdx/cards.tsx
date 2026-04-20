import Link from "next/link";
import type { ReactElement, ReactNode } from "react";

type CardsProps = {
  num?: number;
  children: ReactNode;
};

type CardProps = {
  title: string;
  href: string;
  icon?: ReactElement;
  arrow?: boolean;
  children?: ReactNode;
};

function Cards({ num = 3, children }: CardsProps) {
  const cols =
    num === 1
      ? "grid-cols-1"
      : num === 2
        ? "grid-cols-1 sm:grid-cols-2"
        : num === 4
          ? "grid-cols-1 sm:grid-cols-2 lg:grid-cols-4"
          : "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3";
  return <div className={`not-prose mt-6 grid gap-3 ${cols}`}>{children}</div>;
}

function Card({ title, href, icon, arrow, children }: CardProps) {
  return (
    <Link
      href={href}
      className="nextra-border group flex flex-col gap-1.5 rounded-lg border bg-gray-100 p-4 text-gray-700 no-underline shadow shadow-gray-100 transition-all duration-200 hover:border-gray-300 hover:shadow-md dark:bg-neutral-800 dark:text-gray-300 dark:shadow-none dark:hover:border-neutral-500 dark:hover:bg-neutral-700"
    >
      <span className="flex items-center gap-2 font-semibold text-gray-900 dark:text-gray-50">
        {icon && (
          <span className="inline-flex size-5 items-center justify-center text-current opacity-60 transition-opacity group-hover:opacity-100">
            {icon}
          </span>
        )}
        <span className="truncate">{title}</span>
        {arrow && (
          <span
            aria-hidden="true"
            className="ml-auto opacity-50 transition-transform group-hover:translate-x-0.5 group-hover:opacity-100"
          >
            →
          </span>
        )}
      </span>
      {children && (
        <div className="text-sm leading-relaxed text-gray-600 dark:text-gray-400 [&>p]:m-0">
          {children}
        </div>
      )}
    </Link>
  );
}

Cards.Card = Card;

export { Cards };
