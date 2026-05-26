import type { ReactNode } from "react";
import type { Metadata } from "next";
import { IBM_Plex_Sans, Space_Grotesk } from "next/font/google";
import { siteConfig } from "@/lib/site";
import "./globals.css";

const socialImage = "/thymos-mark.png";

const bodyFont = IBM_Plex_Sans({
  subsets: ["latin"],
  variable: "--font-body",
  weight: ["400", "500", "600", "700"],
});

const displayFont = Space_Grotesk({
  subsets: ["latin"],
  variable: "--font-display",
});

export const metadata: Metadata = {
  metadataBase: new URL(siteConfig.siteUrl),
  title: `${siteConfig.name} | Unified AI Execution Runtime`,
  description: siteConfig.subheadline,
  alternates: {
    canonical: "/",
  },
  openGraph: {
    title: `${siteConfig.name} | Unified AI Execution Runtime`,
    description: siteConfig.subheadline,
    url: siteConfig.siteUrl,
    siteName: siteConfig.name,
    type: "website",
    images: [
      {
        url: socialImage,
        width: 1200,
        height: 630,
        alt: `${siteConfig.name} mark`,
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: `${siteConfig.name} | Unified AI Execution Runtime`,
    description: siteConfig.subheadline,
    images: [socialImage],
  },
  icons: {
    icon: [
      { url: `${siteConfig.basePath}/favicon.ico`, sizes: "any" },
      { url: `${siteConfig.basePath}/icon.png`, type: "image/png" },
    ],
    apple: [
      { url: `${siteConfig.basePath}/apple-icon.png`, sizes: "180x180", type: "image/png" },
    ],
    shortcut: [`${siteConfig.basePath}/favicon.ico`],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={`${bodyFont.variable} ${displayFont.variable}`}>{children}</body>
    </html>
  );
}
