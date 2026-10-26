/** @type {import('tailwindcss').Config} */
export default {
  content: {
    files: [
      './frontend/index.html',
      './frontend/**/*.{vue,js,ts,jsx,tsx}',
    ],
    transform: {
      js: (content) => content.replace(/\/\[[^\n]*?\]\//g, ''),
      ts: (content) => content.replace(/\/\[[^\n]*?\]\//g, ''),
    },
  },
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        /* Orbit v3 Brand */
        brand: {
          DEFAULT: 'var(--palette-brand)',
          hover: 'var(--palette-brand-hover)',
          active: 'var(--palette-brand-active)',
          soft: 'var(--palette-brand-soft)',
          border: 'var(--palette-brand-border)',
        },
        /* Semantic */
        success: {
          DEFAULT: 'var(--palette-success)',
          soft: 'var(--palette-success-soft)',
          border: 'var(--palette-success-border)',
        },
        warning: {
          DEFAULT: 'var(--palette-warning)',
          hover: 'var(--palette-warning-hover)',
          soft: 'var(--palette-warning-soft)',
          border: 'var(--palette-warning-border)',
          bg: 'var(--palette-warning-bg)',
        },
        danger: {
          DEFAULT: 'var(--palette-danger)',
          hover: 'var(--palette-danger-hover)',
          soft: 'var(--palette-danger-soft)',
        },
        /* Text */
        txt: {
          DEFAULT: 'var(--palette-txt)',
          secondary: 'var(--palette-txt-secondary)',
          tertiary: 'var(--palette-txt-tertiary)',
          disabled: 'var(--palette-txt-disabled)',
          link: 'var(--palette-txt-link)',
          tag: 'var(--palette-txt-tag)',
          dark: 'var(--palette-txt-dark)',
          'dark-secondary': 'var(--palette-txt-dark-secondary)',
          'dark-tertiary': 'var(--palette-txt-dark-tertiary)',
        },
        /* Surface */
        surface: {
          DEFAULT: 'var(--palette-surface)',
          hover: 'var(--palette-surface-hover)',
          white: 'var(--palette-surface-white)',
          page: 'var(--palette-surface-page)',
          tag: 'var(--palette-surface-tag)',
          dark: 'var(--palette-surface-dark)',
          'dark-muted': 'var(--palette-surface-dark-muted)',
          'dark-page': 'var(--palette-surface-dark-page)',
        },
        /* Border */
        bdr: {
          light: 'var(--palette-bdr-light)',
          DEFAULT: 'var(--palette-bdr)',
          divider: 'var(--palette-bdr-divider)',
          subtle: 'var(--palette-bdr-subtle)',
          dark: 'var(--palette-bdr-dark)',
          'dark-light': 'var(--palette-bdr-dark-light)',
        },
        /* Overlay */
        overlay: {
          sheet: 'var(--palette-overlay-sheet)',
          dialog: 'var(--palette-overlay-dialog)',
        },
        /* Accent color options (for theme picker) */
        accent: {
          green: 'var(--palette-accent-green)',
          blue: 'var(--palette-accent-blue)',
          purple: 'var(--palette-accent-purple)',
          amber: 'var(--palette-accent-amber)',
          red: 'var(--palette-accent-red)',
          black: 'var(--palette-accent-black)',
        },
      },
      fontFamily: {
        sans: ['-apple-system', 'SF Pro Display', 'Helvetica Neue', 'sans-serif'],
      },
      fontSize: {
        'nano': ['10px', { lineHeight: '1.2' }],
        'micro': ['11px', { lineHeight: '1.3' }],
        'caption': ['12px', { lineHeight: '1.3' }],
        'label': ['13px', { lineHeight: '1.4' }],
        'body': ['14px', { lineHeight: '1.45' }],
        'section': ['15px', { lineHeight: '1.4' }],
        'heading': ['17px', { lineHeight: '1.3' }],
        'title': ['26px', { lineHeight: '1.3' }],
        'status-bar': ['15px', { lineHeight: '1' }],
      },
      fontWeight: {
        regular: '400',
        medium: '500',
        semibold: '600',
        bold: '700',
      },
      borderRadius: {
        'xs': '2px',
        'sm': '4px',
        'md': '6px',
        'DEFAULT': '8px',
        'lg': '10px',
        'xl': '12px',
        '2xl': '14px',
        '3xl': '18px',
        'sheet': '20px',
      },
      spacing: {
        '0.5': '2px',
        '1': '4px',
        '1.5': '6px',
        '2': '8px',
        '2.5': '10px',
        '3': '12px',
        '3.5': '14px',
        '4': '16px',
        '5': '20px',
        '6': '24px',
        '7': '28px',
        '8': '32px',
        '10': '40px',
      },
      boxShadow: {
        'dialog': 'var(--shadow-dialog)',
        'search-focus': 'var(--shadow-search-focus)',
        'segment-active': 'var(--shadow-segment-active)',
      },
      keyframes: {
        'sheet-slide-up': {
          '0%': { transform: 'translateY(100%)' },
          '100%': { transform: 'translateY(0)' },
        },
        'dialog-fade-in': {
          '0%': { opacity: '0', transform: 'scale(0.95)' },
          '100%': { opacity: '1', transform: 'scale(1)' },
        },
        'skeleton-pulse': {
          '0%, 100%': { opacity: '1' },
          '50%': { opacity: '0.4' },
        },
      },
      animation: {
        'sheet-slide-up': 'sheet-slide-up 0.3s cubic-bezier(0.32, 0.72, 0, 1)',
        'dialog-fade-in': 'dialog-fade-in 0.2s ease-out',
        'skeleton-pulse': 'skeleton-pulse 1.5s ease-in-out infinite',
      },
      transitionTimingFunction: {
        DEFAULT: 'ease',
      },
    },
  },
  plugins: [],
}
