import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'XavierDB - Just less than MongoDB',
  description: 'A REST API exposing MongoDB, just less than MongoDB.',
  base: '/',
  cleanUrls: true,
  head: [
    ['link', { rel: 'icon', type: 'image/png', href: '/logo.png' }],
  ],
  themeConfig: {
    logo: '/logo.png',
    socialLinks: [
      { icon: 'github', link: 'https://github.com/ThomasByr/XavierDB' },
    ],
    footer: {
      message: 'Project licence: <a href="https://github.com/ThomasByr/XavierDB/blob/main/LICENSE">MIT</a>',
      copyright: 'Website: <a href="https://github.com/ThomasByr">ThomasByr</a> — all rights reserved',
    },
  },
})
