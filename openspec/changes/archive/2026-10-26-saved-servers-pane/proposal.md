## Why
The SFTP pane opens on a full-page form. The Network pane is better: a tree you browse, with a popup for anything that needs input. Saved SFTP servers should be browsed the same way, and the inline form (which also flickered while Network polls ran) goes away.

## What Changes
- **Saved servers location:** new virtual location `servers:///` lists the saved SFTP profiles as folders (`name`, host and user as detail). It is the tree root of an SFTP pane that has no connection yet.
- **Open a server:** expanding or opening a saved server connects. A remembered password connects silently, otherwise the existing credentials popup asks; the host-key dialog is unchanged.
- **Connect… button** in the SFTP pane header opens a popup (host, port, user, password or key file, start folder, "Save as" name) like the SMB one. Saving is optional.
- **Manage:** right-click a saved server for Connect, Edit and Delete. Edit reopens the popup prefilled.
- **Removed:** the inline "Connect to Server" form and its "Back to Files" button. A typed `sftp://` address and "Open SFTP Pane" from Network use the popup.
- SMB profiles are not listed here; they stay in the SMB connect popup picker.

## Impact
- Code: `fm-vfs` (`Scheme::Servers`), `fm-engine` (servers read from `Prefs.profiles`, `remote.profile.*` reuse, popup state), `fm-ui` (header button, popup, context menu, tree root), `fm-app` (menu).
- Specs: `pane-types` (Remote pane controls), `remote-connections` (profiles listed, popup instead of form).
- No new secrets on disk; profiles are unchanged.
