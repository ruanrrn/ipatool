# ipaTool Design System Contract

## Goal

Make style switching low-cost: future theme/style changes should happen primarily in `frontend/tokens.css`, not by editing scattered component styles.

## Single Source of Truth

- **Allowed design primitives** live in `frontend/tokens.css`
- Outside `frontend/tokens.css`, components should consume tokens via `var(--token-name)`
- Hardcoded colors outside `frontend/tokens.css` are forbidden (`pnpm run lint:tokens`, enforced in CI)
  - `frontend/tailwind.config.js` maps every palette entry to a `--palette-*` token
  - Translucent tints use channel tokens: `rgba(var(--rgb-black), 0.5)`
  - A color that is data rather than style (e.g. the accent picker's options, a
    `<meta name="theme-color">` value) may stay literal on a line marked `color-literal-ok`

## Token Layers

1. **Primitive palette**
   - Radix palette vars imported in `frontend/main.js`
   - Example: `--slate-*`, `--blue-*`, `--red-*`
2. **Semantic tokens**
   - Example: `--color-bg`, `--color-primary`, `--color-danger`, `--color-border`
3. **Component tokens / aliases**
   - Example: `--accent-blue`, `--separator`, `--radius-card`, `--font-size-sm`

## Required Token Domains

### Color
- surface / surface-muted / text / text-muted / border
- primary / success / warning / danger / info
- overlay / mask

### Typography
- `--font-size-xs`
- `--font-size-sm`
- `--font-size-md`
- `--font-size-lg`
- `--font-size-xl`
- `--font-size-2xl`

### Radius
- `--radius-control`
- `--radius-card`
- `--radius-field`
- `--radius-artwork`
- `--radius-artwork-lg`
- `--radius-full`

### Border / Shadow
- `--border-width-thin`
- `--shadow-none`
- `--shadow-elevated-hover`
- `--mask-overlay`

### Spacing / Size
- Existing `--space-1..6` remain the base spacing scale
- Prefer tokenized sizes for high-frequency controls (header icon, artwork, button heights) as the system evolves

## Component Contract

### Button
- Semantic types only: `primary | success | warning | danger | info | default`
- Destructive actions must use `danger`
- Plain variants should still reflect semantic type

### Dialog / Confirm
- Destructive confirms should use unified danger confirm styling
- Overlay should come from tokenized mask values

### Card / Panel / Row
- Surface should come from `--card-bg` / `--surface-muted`
- Border color from `--separator`
- Radius from `--radius-card`

### Input / Select / Textarea
- Radius from `--radius-field`
- Border width/color tokenized
- Font size from size tokens

### Tag / Badge / Status
- Semantic status only
- No ad-hoc colors per page

## Tailwind Usage Rules

Allowed:
- layout utilities (`flex`, `grid`, `items-center`, `justify-between`)
- spacing utilities during transition period
- token-backed arbitrary values like `rounded-[var(--radius-control)]`

Avoid:
- raw arbitrary px values like `rounded-[12px]`, `text-[13px]`
- direct color utilities that bypass tokens for core UI semantics

## Migration Priority

1. Colors
2. Typography sizes
3. Radius
4. Border width
5. Overlay / shadow
6. Spacing / size
7. Tailwind arbitrary cleanup

## Definition of Done

A component is considered tokenized only when:
- colors come from semantic tokens
- typography size comes from font-size tokens
- radius comes from radius tokens
- border width/color comes from border tokens
- overlay/shadow come from tokens when applicable

Then future style switching can happen by editing token definitions rather than component internals.
