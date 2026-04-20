import Link from "next/link";

export default function NotFound() {
  return (
    <div className="min-h-screen flex flex-col items-center justify-center px-6 text-center bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100">
      <div className="font-mono text-7xl font-bold text-cyan-600 dark:text-cyan-400">
        404
      </div>
      <h1 className="mt-4 text-2xl font-bold">Page not found</h1>
      <p className="mt-2 text-neutral-600 dark:text-neutral-400 max-w-md">
        This page moved, never existed, or got renamed between versions.
      </p>
      <div className="mt-8 flex gap-4">
        <Link
          href="/"
          className="px-5 py-2.5 bg-cyan-600 hover:bg-cyan-700 text-white rounded-lg font-medium transition"
        >
          Home
        </Link>
        <Link
          href="/docs/latest"
          className="px-5 py-2.5 border border-black/10 dark:border-white/20 hover:bg-black/5 dark:hover:bg-white/5 rounded-lg font-medium transition"
        >
          Docs
        </Link>
      </div>
    </div>
  );
}
