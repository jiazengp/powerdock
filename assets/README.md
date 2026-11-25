# PowerDock application icon

`PowerDock.ico` is the static application/taskbar/installer icon. The tray icon
continues to render the selected peripheral and its battery fill dynamically.

## Design

The icon is a single full battery with three charge cells on a transparent
background. Its silhouette follows the user-provided Lucide `battery-full`
reference, with a solid graphite body, silver rim and light silver cells.
The design uses neutral colors, restrained two-stop gradients and shallow
front-facing layers to give the battery the weight of a Windows app icon.
There is no background tile, window frame, dashboard, text or external shadow.
A 48px design grid, 6px outer corners, 2.5px cell corners and generous cell
spacing keep the shape soft and legible at small sizes. The dark body defines the icon on light surfaces; the
silver rim and cells define it on dark surfaces.

- Design guidance (consulted 2026-11-15): https://learn.microsoft.com/windows/apps/design/iconography/app-icon-design
- Windows sizes: https://learn.microsoft.com/windows/apps/design/iconography/app-icon-construction

`PowerDock.svg` is the editable source of truth; regeneration never overwrites it.
`PowerDock.png` is a transparent 256px export. The ICO embeds 15 independently
rasterized sizes: 16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128 and 256px,
covering Windows title bars, taskbars and Start at 100–400% scale. The Rust
executable, WinUI executable/window and installer already share this ICO.

## Retained Fluent assets

The previous application's white battery/bolt glyph derived from Microsoft's **Fluent System Icons**,
`Battery Charge / ic_fluent_battery_charge_24_filled.svg`, at commit
`67e6dec1ee956af3ab4c746a2b3bb6a2e5520a44`:
https://github.com/microsoft/fluentui-system-icons/blob/67e6dec1ee956af3ab4c746a2b3bb6a2e5520a44/assets/Battery%20Charge/SVG/ic_fluent_battery_charge_24_filled.svg

The original is retained as `fluent-battery-charge.svg`; its MIT license is
`FLUENT-ICONS-LICENSE.txt`. It is retained for provenance; the new app icon uses
original geometry.

The card's solid charging bolt uses the same project's `Flash / ic_fluent_flash_16_filled.svg`
at the same commit, retained as `fluent-flash-filled.svg`. Its geometry is used
by native WinUI Path shapes, inheriting the percentage's text color and
aligned with each TextBlock's actual baseline.

## Regeneration

Regenerate the PNG and 15-size ICO from the SVG using:

```powershell
uv run scripts/generate-app-icon.py
```

The generator pins resvg-py and Pillow as development tools. Neither Python nor
these packages are included in the application or needed to build it from the
committed icon files.
