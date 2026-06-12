export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // Neutral warm-dark palette modelled after Logic Pro / Bitwig
        // chrome. No blue/violet tint in the grays so the accent stays
        // the only chromatic element on screen.
        'daw-bg': '#1a1a1a',
        'daw-panel': '#222222',
        'daw-surface': '#1e1e1e',
        'daw-control': '#2c2c2c',
        'daw-border': '#3a3a3a',
        // Soft DAW blue — distinct from mute red and solo yellow so the
        // three status colors never collide on a strip.
        'daw-accent': '#4a90d9',
      },
    },
  },
  plugins: [],
};
