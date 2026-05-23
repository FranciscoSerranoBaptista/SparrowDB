import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { source } from '@/lib/source';
import type { ReactNode } from 'react';

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout
      tree={source.pageTree}
      nav={{
        title: (
          <span className="font-semibold text-base tracking-tight">
            SparrowDB
          </span>
        ),
      }}
      sidebar={{
        banner: (
          <div className="rounded-lg border bg-card p-3 text-sm text-muted-foreground">
            <span className="font-medium text-foreground">v0.1</span> — early
            access
          </div>
        ),
      }}
    >
      {children}
    </DocsLayout>
  );
}
