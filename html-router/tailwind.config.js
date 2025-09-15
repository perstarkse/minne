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
    screens: {
      sm: '570px',
      // md: '600px',
      md: '698px',
      lg: '954px',
      xl: '1210px',
      '2xl': '1466px',
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
    themes: [
      {
        light: {
          primary: "#0f172a", // near-black for strong accents
          secondary: "#2563eb", // bold blue accent
          accent: "#f59e0b", // warm amber accent
          neutral: "#111111", // text/outline color
          "base-100": "#fcfaf1", // warm off-white background
          "base-200": "#efede4",
          "base-300": "#e4e1d8",
          info: "#0891b2",
          success: "#16a34a",
          warning: "#eab308",
          error: "#dc2626",
          // neobrutalist geometry
          "rounded-box": "0rem",
          "rounded-btn": "0rem",
          "rounded-badge": "0rem",
          "border-btn": "2px",
          "tab-radius": "0rem",
        },
      },
      {
        dark: {
          primary: "#faf7f2",
          secondary: "#60a5fa",
          accent: "#fbbf24",
          neutral: "#f5f5f4",
          "base-100": "#0f0f10", // near-black canvas
          "base-200": "#171718",
          "base-300": "#1e1f20",
          info: "#22d3ee",
          success: "#22c55e",
          warning: "#fde047",
          error: "#f87171",
          "rounded-box": "0rem",
          "rounded-btn": "0rem",
          "rounded-badge": "0rem",
          "border-btn": "2px",
          "tab-radius": "0rem",
        },
      },
    ],
  },
}
