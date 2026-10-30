# Accessibility

## Keyboard

| Shortcut | Action |
|---|---|
| `Ctrl/⌘ K` | Focus search |
| `Escape` | Close the focused dialog or leave search |
| `Tab` / `Shift+Tab` | Move between controls |

All dialogs are reachable and closable with the keyboard. Focus is always
visible (a 2 px accent outline).

## Screen readers

- Dialogs use `role="dialog"`, `aria-modal`, and a labelled title.
- Progress bars expose `role="progressbar"` with `aria-valuenow` and a name.
- Status is never conveyed by colour alone: every pill carries text.
- Navigation buttons expose `aria-current`/`aria-pressed`; toggles use
  `role="switch"` with `aria-checked`.
- Toasts use `role="status"`.

## Motion and theme

- `prefers-reduced-motion` disables animations and transitions.
- Light/dark themes follow the system by default and can be toggled in the
  header.

## Known gaps

- Dialogs do not trap focus; keyboard users can tab into the background.
- The UI is English-only. `ui.language` is reserved and the app is structured
  for a future string table; Bengali is not shipped yet.
- The dashboard has not been tested with a screen reader on all three
  platforms.
- No automated axe/WCAG checks yet; typecheck and manual review only.
