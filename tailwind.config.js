/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './templates/**/*',
    '!./templates/email/**/*'          
  ],
   theme: {
    extend: {
      typography: {
        DEFAULT: {
          css: {
            maxWidth: '90ch', // Override max-width for all prose instances
          },
        },
      },
    },
  },
  plugins: [
    require("@tailwindcss/typography"),
    require('daisyui')],
  daisyui: {
    themes: ["light", "dark"],
  },
}

