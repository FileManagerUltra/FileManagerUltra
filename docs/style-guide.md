# Dialog style guide

Preferences is the reference for application dialogs. Use this guide for new dialogs
and changes to existing ones, in both dark and light themes.

Preview information summaries reuse this typography and flat section hierarchy
inside the pane, not floating-window chrome. Keep metadata labels muted, values
left-aligned and selectable, and wrap long paths/filenames within the pane.
Volume details belong here rather than in extra Devices sidebar rows. Document,
code, hex and log content retain their renderer-specific typography.
Read-only Preview summaries use compact 6-point row spacing rather than dialog
control heights. Subfolder totals use a dense Name / Size / Files table;
use 5-point row spacing and truncate long folder names to one line with the full
name available on hover.
Give subfolder names all remaining pane width after compact size/count columns,
not the narrow label width used by overview fields.

## Shared implementation

Construct every floating dialog with `fm_ui::dialog_style::window`, passing an open
flag, and call `dialog_style::apply` at the start of its content closure. Use a stable
window ID. The shared constructor supplies the frame, proportional title, title-bar
close button and initial centered placement. It must not be anchored: users can drag
the title bar, and their position is retained across frames and viewport resizing.
Jobs starts at the bottom right instead of the center.

## Typography and hierarchy

- Dialog text is proportional, sized from `Tokens::font_size`; titles are 2 points larger.
  Leave file listings, terminals and document/code views in their existing fonts.
- Use neutral `text` headings, not accent-colored headings. Reserve accents for
  selection, focus and interactive elements.
- Use `dialog_style::section_heading` for flat groups with a thin divider. Do not
  nest settings inside cards. Use a border for an actual list, not every setting.
- Use `dialog_style::hint` for secondary explanations (1 point smaller, `text_muted`).
  Keep errors and warnings explicit using their semantic tokens.
- Monospace is appropriate for exact technical data such as SSH fingerprints, not
  ordinary captions or explanations.

## Layout and controls

- Use the shared frame's token-based padding, 12-point horizontal / 10-point vertical
  spacing and 28-point minimum interactive height.
- Align related labels and inputs in a grid. Include units beside numeric values.
  Give fields accessible labels and retain keyboard navigation.
- Use checkboxes for independent toggles and radios for mutually exclusive choices.
- Keep actions together at the bottom. Long lists and settings scroll inside the
  dialog; do not make Close depend on scrolling to the end of the content.
- Preferences uses a flat, left-aligned topic list, neutral page heading and fixed
  footer. Do not add a sidebar to dialogs that do not need one.
- Use semantic colors from `Tokens` only, with the same hierarchy in both themes.

## Closing and safety

Every dialog has a title-bar close button, including confirmations and subdialogs.
Treat it as Cancel, never as approval or Apply. Cancel must not delete files, execute
programs, change permissions, trust a host key or discard unsaved workspaces.
Reuse existing cancellation commands, including cancelling a pending connection or
conflict. A child file picker closes independently of its parent.

Preferences keeps already-applied changes when closed. Closing Jobs hides the panel
without cancelling jobs or dismissing an update; the status-bar indicator reopens it.
Existing footer actions and Escape behavior remain available.

## Verification

Cover title-bar closing and dragging with interaction tests. Verify cancellation
semantics, position retention, small-window access and that dialog styling does not
leak to the main interface. Refresh only affected snapshots and review both themes.
