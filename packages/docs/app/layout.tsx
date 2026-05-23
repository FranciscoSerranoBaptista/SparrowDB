import { RootProvider } from 'fumadocs-ui/provider';
import type { ReactNode } from 'react';
import './globals.css';

export const metadata = {
  title: {
    template: '%s | SparrowDB',
    default: 'SparrowDB Docs',
  },
  description:
    'Documentation for SparrowDB — an open-source graph-vector database built in Rust.',
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body>
        <RootProvider>{children}</RootProvider>
      </body>
    </html>
  );
}
