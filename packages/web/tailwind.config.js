export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // Neutral WARM-dark chrome modelled after studio hardware and
        // Logic/Bitwig. The grays carry a faint warmth (never blue) so
        // the single steel-blue accent stays the only cool element.
        'daw-bg': '#191817',
        'daw-panel': '#21201e',
        'daw-surface': '#1d1c1b',
        'daw-control': '#2b2a27',
        'daw-border': '#383633',
        'daw-line': '#2e2c29',
        'daw-accent': '#5a9ad4',
        'daw-accent-hi': '#79b3e6',
        // The transport LCD — warm phosphor readout, Logic-style.
        'lcd-bg': '#23251c',
        'lcd-face': '#2a2d21',
        'lcd-text': '#dde6c2',
        'lcd-dim': '#8d9377',
        // Meter ladder.
        'meter-green': '#46c067',
        'meter-amber': '#e3b341',
        'meter-red': '#e5534b',
        // Channel state colors (shared with mute/solo buttons).
        'state-solo': '#e3b341',
        'state-mute': '#e5534b',
      },
      fontFamily: {
        // Native pro-app stacks: UI text rides the system CJK face the
        // OS renders best; every numeric readout is tabular mono.
        ui: [
          '-apple-system',
          'BlinkMacSystemFont',
          'PingFang SC',
          'Hiragino Sans GB',
          'Segoe UI',
          'sans-serif',
        ],
        lcd: ['SF Mono', 'Menlo', 'Monaco', 'JetBrains Mono', 'monospace'],
      },
      boxShadow: {
        lcd: 'inset 0 1px 4px rgba(0,0,0,0.6), inset 0 0 24px rgba(0,0,0,0.25)',
        strip: '0 1px 0 rgba(0,0,0,0.4), inset 0 1px 0 rgba(255,255,255,0.025)',
        raised: '0 1px 2px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.05)',
      },
    },
  },
  plugins: [],
};
