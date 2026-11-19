## ADDED Requirements

### Requirement: PDF documents are inert and bounded
PDF content SHALL be treated as hostile. JavaScript, Launch, remote go-to, import,
submit-form and embedded-file actions SHALL never be performed. Embedded files SHALL
never be extracted or opened. Rendering SHALL use no network and no file other than
the document itself and the renderer's built-in fonts and CMaps. URI links SHALL open
only through the app's explicit link-opening path after a user click. The `pdf-view`
size, pixel, cache and time limits SHALL hold for previews, thumbnails, text
extraction and verification, and the editing parser SHALL only be used for files up
to 64 MiB. CMS signature data SHALL be parsed from at most 64 KiB per signature,
`/ByteRange` values SHALL be bounds-checked before any read, and certificate chains
SHALL be capped at 16 certificates.

#### Scenario: PDF with an auto-run script and an attachment
- **WHEN** a PDF has an OpenAction JavaScript and an embedded executable
- **THEN** previewing, filling, signing and verifying it never run the script or write the attachment anywhere

#### Scenario: Malicious ByteRange
- **WHEN** a signature's `/ByteRange` points past the end of the file or overlaps itself
- **THEN** verification reports Broken (malformed) without reading out of bounds

### Requirement: Signing and document secrets never persist
PDF user passwords, PKCS#12 passwords and private keys SHALL be carried as secret
values that are cleared after use. They SHALL never be written to disk, preferences,
saved workspaces, the diagnostic log or any error or notice text. A command whose
parameters contain them SHALL redact them in every logged or displayed form. Filled
form values SHALL NOT be written to disk before the user saves. Remembered visual
signatures SHALL be opt-in, stored only under the app data folder's `signatures/`
directory with owner-only permissions (`0700`/`0600`, temporary file then rename),
and bounded (8 entries, 8 MiB). PDF preferences SHALL contain only the non-secret fit
and colour modes. The RSA implementation's known timing side channel
(RUSTSEC-2023-0071) is accepted because signing runs locally with no attacker-observable
oracle; this exception SHALL be revisited when a fixed release is available.

#### Scenario: Secrets are absent from errors and logs
- **WHEN** signing fails because of a wrong PKCS#12 password, or a PDF password is wrong
- **THEN** the error, notice and diagnostic log contain neither the password nor any key bytes (unit tests over every PDF error's `Display`)

#### Scenario: Remembered signature files are private
- **WHEN** a signature is remembered on Unix
- **THEN** its folder is `0700` and its file `0600`, and no signature data appears in `prefs.toml`
