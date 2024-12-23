/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './templates/**/*',
    '!./templates/email/**/*'          
  ],
  theme: {
    extend: {},
  },
  plugins: [require('daisyui')],
  daisyui: {
    themes: ["light", "dark"],
  },
}

