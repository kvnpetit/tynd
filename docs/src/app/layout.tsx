import type { Metadata, Viewport } from "next";
import { Head } from "nextra/components";
import { SITE } from "../lib/site";
import "./globals.css";
import "nextra-theme-docs/style.css";

const twitterHandle: string = SITE.social.twitter;
const twitterCreator = twitterHandle
  ? `@${twitterHandle.replace(/^@/, "")}`
  : undefined;

export const metadata: Metadata = {
  metadataBase: new URL(SITE.url),
  title: { default: SITE.title, template: SITE.titleTemplate },
  description: SITE.description,
  applicationName: SITE.name,
  authors: [{ name: SITE.author.name, url: SITE.author.url }],
  creator: SITE.author.name,
  publisher: SITE.author.name,
  keywords: [...SITE.keywords],
  category: SITE.category,
  icons: SITE.icons,
  alternates: { canonical: "/" },
  robots: {
    index: SITE.robots.index,
    follow: SITE.robots.follow,
    googleBot: {
      index: SITE.robots.index,
      follow: SITE.robots.follow,
      "max-image-preview": "large",
      "max-snippet": -1,
      "max-video-preview": -1,
    },
  },
  verification: {
    google: SITE.verification.google || undefined,
    yandex: SITE.verification.yandex || undefined,
    other: SITE.verification.bing
      ? { "msvalidate.01": SITE.verification.bing }
      : undefined,
  },
  openGraph: {
    title: SITE.title,
    description: SITE.description,
    url: SITE.url,
    siteName: SITE.name,
    locale: SITE.ogLocale,
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: SITE.title,
    description: SITE.description,
    creator: twitterCreator,
    site: twitterCreator,
  },
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: SITE.themeColor.light },
    { media: "(prefers-color-scheme: dark)", color: SITE.themeColor.dark },
  ],
  colorScheme: "light dark",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang={SITE.locale} dir="ltr" suppressHydrationWarning>
      <Head />
      <body>{children}</body>
    </html>
  );
}
