## ADDED Requirements

### Requirement: Markdown HTML is inert and bounded
HTML in Markdown SHALL never execute, load styles, scripts, frames or any resource other than images under the Markdown image rules. HTML blocks SHALL be limited to 256 KiB each and nesting depth to 32; attribute values SHALL be length-bounded; exceeded limits SHALL truncate with the existing notice rather than fail.

#### Scenario: Hostile HTML
- **WHEN** Markdown contains deeply nested tags, huge attributes, unterminated tags or entity bombs
- **THEN** the preview stays responsive, memory stays within the caps and nothing is fetched except permitted images
