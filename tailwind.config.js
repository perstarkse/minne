/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './src/server/templates/**/*'
  ],
  theme: {
    extend: {},
  },
  plugins: [require('daisyui')],
  daisyui: {
    themes: ["light", "dark"],
  },
}

