const colors = require('tailwindcss/colors');
const typography = require('@tailwindcss/typography');
const forms = require('@tailwindcss/forms');

const config = {
  content: ['./src/**/*.{html,js,svelte,ts}', './node_modules/svelte-ux/**/*.{svelte,js}'],

  theme: {
    extend: {
      colors: {
        accent: colors.emerald,
        'color-var': 'var(--color)',
      },
    },
  },

  plugins: [typography, require('svelte-ux/plugins/tailwind.cjs')],
};

module.exports = config;
