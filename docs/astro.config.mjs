import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://yuhangch.github.io',
  base: '/panda-reader',
  output: 'static',
  integrations: [
    starlight({
      title: 'Panda Reader',
      description: 'Guides for using Panda Reader and building article plugins.',
      favicon: '/app-icon.png',
      logo: {
        src: './public/app-icon.png',
        alt: 'Panda Reader',
        replacesTitle: false,
      },
      components: {
        SiteTitle: './src/components/SiteTitle.astro',
      },
      defaultLocale: 'en',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/yuhangch/panda-reader',
        },
      ],
      customCss: ['./src/styles/typography.css', './src/styles/starlight.css'],
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Documentation', link: '/docs/' },
            { label: 'Linux support', link: '/docs/linux/' },
          ],
        },
        {
          label: 'Sync',
          items: [{ label: 'Provider setup', link: '/docs/providers/' }],
        },
        {
          label: 'Plugins',
          items: [
            { label: 'Overview', link: '/docs/plugins/' },
            { label: 'Install and update', link: '/docs/plugins/install/' },
            { label: 'Plugin manifest', link: '/docs/plugins/manifest/' },
            { label: 'Rule plugins', link: '/docs/plugins/rules/' },
            { label: 'WASM plugins', link: '/docs/plugins/wasm/' },
            { label: 'Security model', link: '/docs/plugins/security/' },
            { label: 'Testing plugins', link: '/docs/plugins/testing/' },
          ],
        },
      ],
    }),
  ],
});
