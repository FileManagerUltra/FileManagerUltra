## ADDED Requirements

### Requirement: Windows executable icon
Windows builds SHALL embed the branded ICO as an executable resource so
Explorer and installer-created shortcuts use the application icon.

#### Scenario: A Windows shortcut is installed
- **WHEN** the installer creates a shortcut targeting the executable
- **THEN** the shortcut uses the embedded application icon

### Requirement: Linux development desktop identity
The Linux window application ID SHALL match the branded desktop filename and
StartupWMClass. A developer command SHALL register a per-user desktop launcher
and icon without root privileges, respecting XDG_DATA_HOME and quoting the
development binary path.

#### Scenario: Running an uninstalled development build
- **WHEN** a developer runs `cargo xtask desktop-install` and restarts the app
- **THEN** the desktop can resolve the running window to its branded launcher and icon
