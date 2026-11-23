## ADDED Requirements

### Requirement: PDFs render page by page in the Preview pane
A Preview pane showing a PDF SHALL render its pages with a pure-Rust renderer, as a
continuously scrolling, virtualized column of pages separated by a themed gap. Page
rotation (`/Rotate`) and crop boxes SHALL be honoured, and form fields, annotations
and signature appearances SHALL be drawn. Only visible pages, plus one page before and
one after, SHALL be rendered. Rendering SHALL happen off the UI thread on a dedicated
PDF pool, not the file-job lanes. While a page renders, a placeholder of the correct
size SHALL keep the layout stable. Local, SMB and SFTP files SHALL behave the same.
The file information (size, PDF version, page count, title, author, producer,
encryption) SHALL remain available as an information section.

#### Scenario: Open a multi-page PDF
- **WHEN** a 40-page PDF is selected in the source Files pane
- **THEN** the first pages appear in the Preview without blocking any frame
- **AND** scrolling renders further pages on demand while earlier off-screen pages may be evicted from the cache

#### Scenario: Remote PDF
- **WHEN** a PDF on an SFTP or SMB share is previewed
- **THEN** it renders exactly as the same file does locally

#### Scenario: Page that cannot render
- **WHEN** one page of a document fails to render
- **THEN** that page shows an error tile and the other pages render normally

### Requirement: PDF navigation
The Preview toolbar SHALL show the current page and page count ("3 / 12") in an
editable field that accepts a page number or a page label. It SHALL also have
previous/next page buttons, a fit-mode control (Fit width, Fit page, 100%) and a
page-colour control. These SHALL fold into the View options menu in narrow panes.
While the Preview has keyboard focus, PageUp/PageDown, Home/End and the arrow keys
SHALL scroll or move between pages. No new application-wide shortcut SHALL be
introduced. Fit width SHALL be the default. The fit mode and colour mode SHALL be
remembered in preferences.

#### Scenario: Jump to a page
- **WHEN** the user types `7` in the page field and presses Enter
- **THEN** page 7 is scrolled to the top of the view
- **AND** an out-of-range number or unknown label is rejected visibly and the position is unchanged

#### Scenario: Keys in a focused PDF preview
- **WHEN** the PDF preview has focus and the user presses PageDown
- **THEN** the view moves to the next page
- **AND** Ctrl+Shift+= still zooms the whole application, as in every pane

### Requirement: PDF zoom follows the pane zoom
The pane-local Preview magnification (50–400%) SHALL apply on top of the fit mode.
Choosing a fit mode SHALL reset additional magnification, as for images. Pages SHALL
be re-rendered sharp at the new scale; until then, the previous rendering SHALL be
shown scaled. Pages that would exceed 16 megapixels SHALL be rendered as tiles of the
visible area. Zoom SHALL NOT raise file or decoder limits.

#### Scenario: Deep zoom stays sharp
- **WHEN** an A4 page is shown at 400% on a high-DPI display
- **THEN** text is rendered sharp from tiles covering the visible area, without allocating a full-page bitmap above the pixel cap

### Requirement: Outline and links
When a PDF has an outline (bookmarks), the toolbar SHALL offer an outline list in the
pane showing the nested titles (collapsible). Choosing an entry SHALL navigate to its
destination. Internal links and named destinations SHALL navigate when clicked. URI
links SHALL show their target on hover and open only through the app's explicit
link-opening path. Launch, remote go-to (GoToR), JavaScript, submit-form and
embedded-file actions SHALL NOT be performed.

#### Scenario: Follow an internal link
- **WHEN** the user clicks a table-of-contents link to page 12
- **THEN** page 12 is shown and the page field reads 12

#### Scenario: Launch action is inert
- **WHEN** a link carries a Launch action to an executable
- **THEN** nothing is started and hovering shows that the link is not supported

### Requirement: Page colours for low-vision reading
The page-colour control SHALL offer *Original*, *Dark* (luminance inverted, hue kept)
and *High contrast* (page text and background mapped to the theme's text and
background colours). Modes SHALL be applied to rendered pixels, never written into
the document, and SHALL behave the same for every PDF. Gaps, borders and the
selection and Find highlights SHALL use theme tokens and stay visible in all three
modes and in both themes.

#### Scenario: Dark pages in the dark theme
- **WHEN** the dark theme is active and the user chooses Dark pages
- **THEN** page backgrounds become dark, text becomes light, photos keep recognizable colours, and saving the document writes no colour change

### Requirement: Find and copy text in PDFs
Pane-local Find (`preview.find.*`) SHALL work in PDFs over text extracted from the
pages, with the existing query, case, whole-word and regex options, the
current/total counter and Previous/Next. The current match SHALL be scrolled into
view and highlighted on the page. Text SHALL be selectable by dragging on a page and
copyable, in reading order per page. Select all (`preview.text.select_all`) SHALL
select the document's extracted text within the existing copy limits. Extraction
SHALL run off the UI thread with the existing Find bounds. Pages without extractable
text (scans) SHALL be reported as "no searchable text" instead of matching nothing
silently.

#### Scenario: Find across pages
- **WHEN** the user searches for "invoice" in a 30-page PDF
- **THEN** the counter shows the total over all pages and Next moves to matches on later pages, wrapping at the end

#### Scenario: Scanned document
- **WHEN** a PDF consisting only of scanned images is searched
- **THEN** Find reports that the document has no searchable text

### Requirement: Password-protected PDFs
A PDF that needs a user password SHALL show a password prompt in the Preview instead
of pages. A wrong password SHALL be reported without revealing it, and the user can
retry. The password SHALL be held only in memory for that pane's open document, never
stored, logged or shown in errors. A PDF that opens with an empty user password but
restricts permissions SHALL render normally and show its restrictions in the
information section.

#### Scenario: Unlock a document
- **WHEN** an encrypted PDF is selected and the user enters the correct password
- **THEN** its pages render, and selecting another file forgets the password

#### Scenario: Wrong password
- **WHEN** a wrong password is entered
- **THEN** the prompt says it is incorrect and the error text does not contain the entered value

### Requirement: Hostile PDFs are bounded
Viewing SHALL be limited to files of at most 256 MiB (larger files show the
information card with the reason). Render output SHALL be at most 16 megapixels per
page or tile, and the PDF render cache at most 192 MiB. A page render exceeding 10
seconds SHALL be abandoned with an error tile. After two abandoned renders the
document SHALL stop rendering and say it is too complex to preview. Renderer panics
SHALL be contained as errors. Embedded fonts, images and streams SHALL be decoded only
within these bounds. No network access or external file SHALL be used to render.

#### Scenario: Pathological page
- **WHEN** a page's content takes longer than 10 seconds to render
- **THEN** an error tile replaces it, the UI stays responsive and other pages keep rendering

#### Scenario: Huge page box
- **WHEN** a page declares a 14 400 × 14 400 pt media box and is zoomed in
- **THEN** only visible tiles within the pixel cap are rendered

### Requirement: PDF view commands
The PDF view actions SHALL be commands: `preview.pdf.page`, `preview.pdf.next`,
`preview.pdf.prev`, `preview.pdf.first`, `preview.pdf.last`, `preview.pdf.fit`,
`preview.pdf.colors`, `preview.pdf.outline.toggle`, `preview.pdf.outline.go`,
`preview.pdf.link`, `preview.pdf.visible`, `preview.pdf.password` and
`preview.pdf.signatures.toggle`, each with ids, labels, menu paths and params docs. They SHALL be disabled
unless the target Preview shows a PDF, and SHALL reject malformed or out-of-range
parameters with an error and no state change.

#### Scenario: Bad page parameter
- **WHEN** `preview.pdf.page` is run with `{"page": -1}`, a string longer than the label limit, or no PDF shown
- **THEN** it returns an error and the view is unchanged
