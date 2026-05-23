import { createPreset } from 'fumadocs-ui/tailwind-plugin';
import type { Config } from 'tailwindcss';

const config: Config = {
  darkMode: 'class',
  content: [
    // Fumadocs
    './node_modules/fumadocs-ui/dist/**/*.js',
    './node_modules/fumadocs-core/dist/**/*.js',
    // App
    './app/**/*.{ts,tsx}',
    './components/**/*.{ts,tsx}',
    './content/**/*.{md,mdx}',
    './mdx-components.{ts,tsx}',
    // Mintlify components
    './node_modules/@mintlify/components/dist/**/*.js',
  ],
  presets: [createPreset()],
};

export default config;
