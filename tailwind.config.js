/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './templates/**/*',
    '!./templates/email/**/*'          
  ],
   theme: {
    container: {
      padding: {
        DEFAULT: '1rem',
        sm: '2rem',
        lg: '4rem',
        xl: '5rem',
        '2xl': '6rem',
      },      
    },
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

