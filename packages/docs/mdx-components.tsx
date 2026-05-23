import defaultMdxComponents from 'fumadocs-ui/mdx';
import { Callout } from 'fumadocs-ui/components/callout';
import { Accordion, Accordions } from 'fumadocs-ui/components/accordion';
import { Tab, Tabs } from 'fumadocs-ui/components/tabs';
import type { MDXComponents } from 'mdx/types';

export function useMDXComponents(components: MDXComponents): MDXComponents {
  return {
    ...defaultMdxComponents,
    ...components,
    // Callout aliases so MDX authors can write <Note>, <Warning>, <Tip>
    Note: (props) => <Callout type="info" {...props} />,
    Warning: (props) => <Callout type="warn" {...props} />,
    Tip: (props) => <Callout type="info" title="Tip" {...props} />,
    Danger: (props) => <Callout type="error" {...props} />,
    Callout,
    Accordion,
    Accordions,
    Tab,
    Tabs,
  };
}
