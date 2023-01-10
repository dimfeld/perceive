const typography = require('@tailwindcss/typography');
const forms = require('@tailwindcss/forms');

const config = {
  content: ['./src/**/*.{html,js,svelte,ts}'],

  theme: {
    extend: {},
  },

  plugins: [forms, typography, require('svelte-ux/plugins/tailwind.cjs')],
};

module.exports = config;
