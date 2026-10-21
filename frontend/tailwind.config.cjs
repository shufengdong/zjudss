/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        brand: {
          50: '#eef1ff',
          100: '#e0e6ff',
          200: '#c7d0fe',
          300: '#a5b3fd',
          400: '#8090fb',
          500: '#4d6bfe',
          600: '#3d52e8',
          700: '#3340c4',
          800: '#2b349e',
          900: '#28307d',
        },
      },
    },
  },
  plugins: [require('@tailwindcss/typography')],
}
