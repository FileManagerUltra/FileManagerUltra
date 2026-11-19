## ADDED Requirements

### Requirement: Fill & Sign mode
A Preview showing a PDF with interactive form fields or signature fields SHALL offer
a **Fill & Sign** toolbar button (`pdf.form.edit`). For PDFs without fields, the
button SHALL still offer signing. Fill mode SHALL be unavailable, with the reason
shown, when the document's permissions forbid filling, a certification signature
forbids changes, the document is larger than 64 MiB, or the editing parser cannot
open it. Viewing SHALL be unaffected in all these cases.

#### Scenario: Restricted document
- **WHEN** a PDF whose permissions forbid form filling is shown
- **THEN** Fill & Sign is disabled and its tooltip explains the restriction

### Requirement: Supported form fields
Fill mode SHALL offer editable controls on top of each field's rectangle at the
current zoom: single-line, multi-line, comb and password (masked) text fields;
checkboxes; radio groups; dropdowns (with free text when the field allows it); and
single- and multi-select list boxes. Read-only fields SHALL NOT be editable. Maximum
length and the choice options SHALL be enforced. Tab and Shift+Tab SHALL move through
fields in the page's tab order, then the next page. Push buttons SHALL be shown but do
nothing. Values SHALL be set through `pdf.form.set {field, value}` using the field's
fully qualified name. Invalid values SHALL be rejected with an error and no change.

#### Scenario: Fill a text field and a checkbox
- **WHEN** the user types a name into a text field and ticks a checkbox
- **THEN** both show the new values immediately, rendered as they will be saved

#### Scenario: Value outside the options
- **WHEN** `pdf.form.set` gives a dropdown a value that is not one of its options and the field does not allow free text
- **THEN** it returns an error and the field keeps its value

### Requirement: Filled fields look the same in every viewer
Saving SHALL regenerate the appearance of every changed field from its default
appearance (font, size including auto-size, colour, alignment, comb and multi-line
layout). Checkboxes and radio buttons SHALL switch to their existing on/off
appearances. The document SHALL NOT rely on viewers regenerating appearances
(`NeedAppearances` is never set). Characters the field's font cannot show SHALL be
refused when typed, with a message naming the character, instead of being saved as
missing glyphs.

#### Scenario: Reopen in another viewer
- **WHEN** a filled and saved form is opened in another PDF viewer
- **THEN** every filled value appears with the field's font and alignment

#### Scenario: Unsupported character
- **WHEN** the user types a character the field's font cannot encode
- **THEN** it is not inserted and an inline message names the character

### Requirement: Scripts and XFA
Form JavaScript (calculate, format, validate, keystroke and other actions) SHALL NOT
run. Fields that carry such scripts SHALL show a note that calculated values are not
updated. XFA-only forms SHALL show an explanation and no fill mode. Hybrid forms SHALL
be filled through their AcroForm fields, and the saved copy SHALL drop the XFA data,
telling the user once when saving.

#### Scenario: Hybrid XFA form
- **WHEN** the user fills and saves a form that has both XFA and AcroForm data
- **THEN** the saved copy has no XFA entry, the user is told why, and other viewers show the filled AcroForm values

### Requirement: Required fields and reset
On save, required fields that are still empty SHALL be listed as a warning that the
user can dismiss to save anyway. `pdf.form.reset` SHALL restore every field to its
value when the document was loaded.

#### Scenario: Save with an empty required field
- **WHEN** the user saves while a required field is empty
- **THEN** a warning names the field, and choosing Save anyway writes the file

### Requirement: Saving preserves the original
`pdf.save` SHALL write a copy next to the original, named `<name> (filled).pdf` or
`<name> (signed).pdf` (with ` 2`, ` 3` … on collision). `pdf.save_as {uri}` SHALL
write to a location chosen with the path picker, on any backend. `pdf.save_overwrite`
SHALL replace the original only after a confirmation, and SHALL refuse if the file
changed on disk since it was loaded. Every save SHALL append an incremental update to
the original bytes, so earlier revisions and existing signatures stay intact. Writes
SHALL go through the file-job lanes with the existing temporary-file-then-rename
write, conflict prompts and progress. After saving, the Preview SHALL show the saved
file. Encrypted documents SHALL be saved encrypted with their existing protection,
or filling SHALL be disabled for them with an explanation.

#### Scenario: Default save next to the original
- **WHEN** the user fills `form.pdf` and presses Save
- **THEN** `form (filled).pdf` is created next to it and `form.pdf` is byte-for-byte unchanged

#### Scenario: Overwrite after an outside change
- **WHEN** the original file was modified by another program after loading and the user chooses Overwrite
- **THEN** the save is refused with an explanation and nothing is written

#### Scenario: Failed write
- **WHEN** the target disk is full or the share drops during the save
- **THEN** no partial file replaces the target, the error is shown, and the unsaved changes are kept

### Requirement: Unsaved changes are never lost silently
While a Preview has unsaved form values or placed signatures, it SHALL stay on that
document instead of following the source Files pane, and its toolbar SHALL show
"Unsaved changes" with Save, Save As and Discard (`pdf.discard`). Closing the pane or
quitting the application SHALL ask Save / Don't save / Cancel. Unsaved values SHALL
NOT be written anywhere on disk before the user saves.

#### Scenario: Cursor moves during filling
- **WHEN** the user has filled two fields and moves the cursor in the source Files pane
- **THEN** the Preview keeps showing the form with its values, and the toolbar shows Unsaved changes

#### Scenario: Quit with unsaved values
- **WHEN** the user quits with unsaved form values
- **THEN** the unsaved-changes prompt appears, and Cancel keeps the application open with the values intact

### Requirement: Form and save commands
The form and save actions SHALL be commands: `pdf.form.edit`, `pdf.form.set`,
`pdf.form.reset`, `pdf.save`, `pdf.save_as`, `pdf.save_overwrite` and `pdf.discard`,
each with ids, labels, menu paths
and params docs, disabled when not applicable, and SHALL return an error without
changing state for malformed parameters, unknown field names, oversized values (over
32 KiB) or invalid target URIs.

#### Scenario: Unknown field
- **WHEN** `pdf.form.set` names a field that does not exist
- **THEN** it returns an error and no value changes
