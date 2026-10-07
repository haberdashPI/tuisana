import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'tuisana',
  description: 'A keyboard-driven terminal UI for Asana projects and tasks.',
  // GitHub Pages serves this project site from /tuisana/.
  base: '/tuisana/',
  lastUpdated: true,
  cleanUrls: true,
  themeConfig: {
    nav: [
      { text: 'Getting started', link: '/getting-started' },
      { text: 'Configuration', link: '/config/' },
      { text: 'Reference', link: '/reference/dates' },
    ],
    sidebar: [
      {
        text: 'Introduction',
        items: [
          { text: 'What tuisana is', link: '/' },
          { text: 'Getting started', link: '/getting-started' },
        ],
      },
      {
        text: 'Configuration',
        items: [{ text: 'The config file', link: '/config/' }],
      },
      {
        text: 'Settings you edit',
        collapsed: false,
        items: [
          { text: 'Authentication', link: '/config/auth' },
          { text: 'Appearance', link: '/config/appearance' },
          { text: 'Key bindings', link: '/config/keybindings' },
          { text: 'Editing behaviour', link: '/config/editing' },
        ],
      },
      {
        text: 'Settings tuisana writes',
        collapsed: false,
        items: [
          { text: 'Overview', link: '/config/managed' },
          { text: 'Named filter sets', link: '/config/filter-sets' },
          { text: 'Saved view state', link: '/config/view' },
          { text: 'Project visibility', link: '/config/projects' },
          { text: 'Gantt colours', link: '/config/gantt' },
        ],
      },
      {
        text: 'Reference',
        items: [
          { text: 'Date expressions', link: '/reference/dates' },
          { text: 'Commands', link: '/reference/commands' },
          { text: 'Config format versions', link: '/reference/migrations' },
        ],
      },
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/haberdashPI/tuisana' },
    ],
    editLink: {
      pattern: 'https://github.com/haberdashPI/tuisana/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },
    search: { provider: 'local' },
    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © David F Little',
    },
  },
})
