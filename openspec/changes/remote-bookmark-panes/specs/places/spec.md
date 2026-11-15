## ADDED Requirements

### Requirement: Remote places open in their own pane
`place.open {uri, pane?}` SHALL open a place (Places click, the Places context menu's Open, Go → Places). For an `smb://` or `sftp://` place opened from a pane that is not a Network pane (SMB) or SFTP pane (SFTP), it SHALL use an existing pane of that kind or else split the pane and make the new half that kind, focus it and open the place there, starting the sign-in when the server is not connected. The pane the place was opened from SHALL keep its location and folder tree. Other places SHALL open in the pane they were opened from. Bad params SHALL return an error and change nothing.

#### Scenario: SMB bookmark from a Files pane
- **WHEN** the user clicks the bookmark `\\192.168.0.50` in a Files pane at `C:\Users\alice` and no Network pane exists
- **THEN** a Network pane opens beside it, the sign-in dialog appears for 192.168.0.50, and after signing in that pane shows the server
- **AND** the Files pane still shows `C:\Users\alice` with its tree

#### Scenario: A Network pane exists
- **WHEN** a Network pane is already open and the user clicks an SMB bookmark in a Files pane
- **THEN** the bookmark opens in that Network pane and no new pane is created

#### Scenario: Local bookmark
- **WHEN** the user clicks a local bookmark in a Files pane
- **THEN** that pane navigates to it, as before
