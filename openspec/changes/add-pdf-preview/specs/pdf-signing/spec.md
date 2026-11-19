## ADDED Requirements

### Requirement: Create a visual signature
The signature dialog SHALL offer three ways to make a visual signature. **Draw** uses
a pad that takes mouse, touch or pen strokes and has Clear and Undo-stroke. **Type**
renders a name in a bundled signature font. **Image** loads a PNG or JPEG chosen with
the path picker, with an option to make a white background transparent. Drawn and
typed signatures SHALL be written to the PDF as vector paths, without embedding a
font. Images SHALL be limited to 8 MiB and 4 megapixels, and strokes to 64 strokes of
2048 points. The dialog SHALL be usable in both themes and show the signature on a
page-coloured background.

#### Scenario: Typed signature
- **WHEN** the user types "Jane Doe" on the Type tab and accepts
- **THEN** the signature is ready to place and, once saved, appears in other viewers without the font being embedded

#### Scenario: Oversized image
- **WHEN** the chosen image is larger than 4 megapixels
- **THEN** it is refused with the reason and the dialog stays open

### Requirement: Place visual signatures
A visual signature SHALL be placed either by clicking an empty signature field (it
fills the field's rectangle, keeping its aspect ratio) or by dragging a rectangle
anywhere on a page. Until saved, a placed signature SHALL be movable, resizable and
removable. Several signatures MAY be placed before one save. A visual-only signature
SHALL be added to the page content of the saved copy, and one in a signature field
SHALL become that field's appearance. If the document already has digital signatures,
the user SHALL be told before saving that those signatures will report changes after
signing.

#### Scenario: Sign anywhere on a page
- **WHEN** the user drags a rectangle at the bottom of page 3 and places a drawn signature there
- **THEN** the signature appears at that spot and can be moved and resized until Save

#### Scenario: Visual signature on a signed document
- **WHEN** the document already carries a valid digital signature and the user places a visual-only signature
- **THEN** a notice explains that the existing signature will show "changed after signing" before the save proceeds

### Requirement: Saved signatures are opt-in and private
"Remember this signature" SHALL be off by default. Remembered signatures SHALL be
stored only in the application data folder, outside the preferences file, with
owner-only permissions (`0700` folder, `0600` files). At most 8 can be stored, 8 MiB in
total, and each can be deleted from the dialog. They SHALL be offered on the Draw,
Type and Image tabs for reuse. Corrupt or oversized entries SHALL be skipped with a
notice.

#### Scenario: Reuse a remembered signature
- **WHEN** the user ticked Remember for a drawn signature and later opens the dialog for another PDF
- **THEN** that signature is offered for reuse, and deleting it removes its file

### Requirement: Digital signature with a certificate file
The dialog's **Digital** tab SHALL sign with a PKCS#12 file (`.p12` or `.pfx`, at most
1 MiB, on any backend) and its password. Optional fields: reason, location and contact
(256 characters each), a visible appearance (a visual signature, the signer's name and
date, or none), and the signature field or page rectangle to use. Supported keys are
RSA (2048–8192 bits) and ECDSA P-256/P-384, with SHA-256 (SHA-384 for P-384).
Signing SHALL produce a PAdES baseline B-B signature (`ETSI.CAdES.detached`, with the
signing certificate attribute and the certificate chain from the file) as the last
incremental update of the save. Any form values and visual signatures go in the same
save, before it. An expired certificate or one whose key usage forbids signing SHALL
be refused with the reason. A document can be signed again, and every signature SHALL
keep verifying.

#### Scenario: Sign a filled form
- **WHEN** the user fills a form, chooses Digital, selects a valid `.p12`, enters its password and saves
- **THEN** the saved copy contains the filled values and a signature that this app reports as Valid
- **AND** the original file is unchanged

#### Scenario: Wrong certificate password
- **WHEN** the password does not decrypt the certificate file
- **THEN** signing fails with "incorrect password or unreadable certificate", nothing is written, and the error contains neither the password nor key material

#### Scenario: Second signature
- **WHEN** a document signed by one person is signed by a second person
- **THEN** both signatures verify, and the first is reported as Valid, changed after signing (signature added)

### Requirement: Certificate secrets stay in memory, briefly
The certificate password SHALL be passed as a secret value, used only to decrypt the
file during one signing operation, and not remembered between signatures. The
decrypted private key SHALL be held in memory that is cleared when the operation ends,
whether it succeeds or fails. Neither SHALL be written to disk, to preferences, to
logs, to the diagnostic log, or into any error text.

#### Scenario: Password is not reused
- **WHEN** the user signs two documents in a row with the same certificate
- **THEN** the password is asked for both times

### Requirement: Verify existing signatures
When a PDF has signature fields, the toolbar SHALL show a signature badge ("2
signatures") that opens a signatures panel. Each signature SHALL be listed with
signer name, signing time as claimed, reason and location when present, issuer, serial
and SHA-256 certificate fingerprint, and one status:
- **Valid**: the signed bytes are intact and the signature covers the whole file.
- **Valid, changed after signing**: the signed revision is intact and later changes
  exist; they are summarized (form fields filled, signatures added, other changes).
- **Broken**: digest or signature mismatch, malformed structure, or an unsupported
  algorithm (named).
Weak algorithms (SHA-1, RSA under 2048 bits) SHALL be flagged on any status. Each
entry SHALL state that the signer identity was not checked against a trust list.
Verification SHALL be offline and run off the UI thread within 5 seconds per
document. If verification takes longer, the panel SHALL say so.

#### Scenario: Tampered document
- **WHEN** a signed PDF has a byte changed inside its signed range
- **THEN** the panel reports Broken with "document content does not match the signature"

#### Scenario: Form filled after signing
- **WHEN** a signed PDF was later filled and saved incrementally
- **THEN** the panel reports Valid, changed after signing, listing form field changes

### Requirement: Signing commands
The signing actions SHALL be commands: `pdf.sign.visual`, `pdf.sign.place`,
`pdf.sign.move`, `pdf.sign.remove`, `pdf.sign.digital`, `pdf.sign.saved.store` and
`pdf.sign.saved.delete`, each with ids, labels, menu paths and params docs, disabled when not applicable.
They SHALL return an error without changing state for malformed parameters: page or
rectangle out of the page box, oversized stroke or image data, unknown signature ids,
reason/location/contact over 256 characters, or a certificate URI that is not a file.

#### Scenario: Rectangle outside the page
- **WHEN** `pdf.sign.place` is given a rectangle outside page 1's box
- **THEN** it returns an error and nothing is placed
