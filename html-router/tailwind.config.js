/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './templates/**/*',
  ],
   theme: {
    container: {
      padding: {
        DEFAULT: '10px',
        sm: '2rem',
        lg: '4rem',
        xl: '5rem',
        '2xl': '6rem',
      },      
    },
    extend: {
      fontFamily: {
        satoshi: ['Satoshi', 'sans-serif'],
      },
      typography: {
        DEFAULT: {
          css: {
            maxWidth: '90ch', // Override max-width for all prose instances
          },
        },
      },
    },
  },
  daisyui: {
    themes: ["light", "dark"],
  },
}

