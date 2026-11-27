## ADDED Requirements

### Requirement: Locations inside archives have a display form
The display form of a location inside an archive SHALL be the display form of the archive file followed by the inner path with the platform's separator (for example `/home/me/site.zip/www` or `C:\Users\me\site.zip\www`). Typed into the address bar, such a form SHALL be resolved from the left, entering each component that is an archive file, up to the nesting limit. Stored addresses SHALL stay canonical (`archive://` with the archive's own address stored beside it).

#### Scenario: Typing a path into a zip
- **WHEN** the user types `/home/me/site.zip/www` into the address bar
- **THEN** the pane shows the `www` folder inside `site.zip`

#### Scenario: Display on Windows
- **WHEN** a pane on Windows shows `www` inside `C:\Users\me\site.zip`
- **THEN** the address bar shows `C:\Users\me\site.zip\www`
