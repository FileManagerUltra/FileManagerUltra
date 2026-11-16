## ADDED Requirements

### Requirement: Diagnostic log is private and bounded
The diagnostic log SHALL never contain file names, paths, URIs, host names, user names, command parameters or credentials: events carry command ids, numbers and fixed labels only, and records from the `log` crate are accepted only from the graphics stack (`egui_glow` at debug level and above; `eframe`, `egui_winit`, `winit` and `glutin` at warning level and above). The log folder SHALL be created owner-only (`0700`) and log files owner-only (`0600`) on Unix. Size and rate are capped (capability `diagnostics`).

#### Scenario: Commands with paths
- **WHEN** the user opens a file from an SMB share while the log is on
- **THEN** the log shows the command id (for example `open.entry`) and never the share, host, user or file name
