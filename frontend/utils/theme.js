export function applyAccentColor(color) {
  const root = document.documentElement
  root.style.setProperty('--color-primary', color)

  const hover = darkenColor(color, 0.1)
  const active = darkenColor(color, 0.15)
  root.style.setProperty('--color-primary-hover', hover)
  root.style.setProperty('--color-primary-active', active)

  const soft = hexToRgba(color, 0.08)
  const softBorder = hexToRgba(color, 0.3)
  root.style.setProperty('--color-primary-soft', soft)
  root.style.setProperty('--color-primary-border', softBorder)
}

export function hexToRgba(hex, alpha) {
  const r = parseInt(hex.slice(1, 3), 16)
  const g = parseInt(hex.slice(3, 5), 16)
  const b = parseInt(hex.slice(5, 7), 16)
  return `rgba(${r}, ${g}, ${b}, ${alpha})` // color-literal-ok: builds a color value
}

export function darkenColor(hex, amount) {
  let r = parseInt(hex.slice(1, 3), 16)
  let g = parseInt(hex.slice(3, 5), 16)
  let b = parseInt(hex.slice(5, 7), 16)
  r = Math.max(0, Math.round(r * (1 - amount)))
  g = Math.max(0, Math.round(g * (1 - amount)))
  b = Math.max(0, Math.round(b * (1 - amount)))
  const toHex = (c) => c.toString(16).padStart(2, '0')
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`
}
