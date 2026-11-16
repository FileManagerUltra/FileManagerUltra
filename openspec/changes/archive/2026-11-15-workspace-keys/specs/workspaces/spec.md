## MODIFIED Requirements

### Requirement: Workspace tabs
The window SHALL show a tab bar of open workspaces. Each workspace owns its full pane layout; switching tabs MUST NOT disturb the others, and background terminals keep running. Closing the last tab replaces it with a fresh default.

#### Scenario: Switch keeps state
- **WHEN** the user changes a pane in tab 1, opens tab 2 and returns to tab 1
- **THEN** tab 1's layout is unchanged and pane ids never clash across tabs

#### Scenario: Shortcuts
- **WHEN** Ctrl+1..9/0 is pressed
- **THEN** the Nth open tab becomes active; Ctrl+T / Ctrl+W add / close, and are ignored while a terminal pane has focus
- **AND** while Preview has focus, Ctrl+0 resets preview zoom instead of switching to tab 10; Ctrl+1..9 still switch tabs
- **AND** text fields and dialogs retain keyboard ownership; macOS uses Cmd for these application shortcuts

#### Scenario: Digit keys, whatever they type
- **WHEN** Ctrl+Shift+1..9 is pressed (Shift+1 types `!` on US layouts), or Ctrl plus a digit key on a layout where digits need Shift (AZERTY)
- **THEN** the tab of that digit key becomes active, exactly as with Ctrl+1..9
- **AND** Ctrl+Shift+0 keeps resetting the zoom

#### Scenario: Bad params
- **WHEN** `workspace.switch {n: 0}` or an out-of-range tab runs
- **THEN** the command returns an error and nothing changes

## ADDED Requirements

### Requirement: Switching tabs with Ctrl+Tab
Ctrl+Tab SHALL run `workspace.next` and Ctrl+Shift+Tab `workspace.prev` (Cmd on macOS keeps Ctrl here, as browsers do), including while a terminal pane has focus; an open dialog or text field keeps the keys (Preferences uses them for its topics). The preference `workspace_tab_order` SHALL choose the order: `recent` (default) or `cycle`. With `recent`, the first Ctrl+Tab activates the most recently used other tab; pressing Tab again while Ctrl stays held moves to the next older one (Shift goes back), and releasing Ctrl makes the tab reached the most recently used one; switching by any other means also counts as use. With `cycle`, Ctrl+Tab activates the tab to the right and Ctrl+Shift+Tab the tab to the left, wrapping around. With one tab both commands do nothing and are disabled.

#### Scenario: Back and forth
- **WHEN** order is `recent`, the user works in tab 3, switches to tab 1 with Ctrl+1, then presses and releases Ctrl+Tab
- **THEN** tab 3 is active; pressing and releasing Ctrl+Tab again returns to tab 1

#### Scenario: Holding Ctrl walks further back
- **WHEN** order is `recent`, tabs were used in the order 2, 4, 1 (1 active), and the user holds Ctrl and presses Tab twice
- **THEN** tab 2 is active after the second press, and after releasing Ctrl the recent order is 2, 1, 4

#### Scenario: Cycle order
- **WHEN** order is `cycle`, the last tab is active and Ctrl+Tab is pressed
- **THEN** the first tab becomes active; Ctrl+Shift+Tab from the first tab activates the last

#### Scenario: Bad params
- **WHEN** `workspace.next` or `workspace.prev` receives malformed params, or `workspace_tab_order` is set to an unknown value
- **THEN** the command returns an error and nothing changes
