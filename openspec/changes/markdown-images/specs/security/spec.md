## MODIFIED Requirements

### Requirement: Previews are untrusted-input safe
File content shown in a Preview SHALL be treated as hostile: bounded reads, header checks before allocation, no network access except opt-in remote Markdown images as defined below, no execution of embedded content, decoder panics contained, and decoder child processes (video) started without a shell and with fixed arguments, the file passed as a single argument after `--`/via `file:` protocol guard.

Markdown images SHALL be limited to 8 MiB of source and 100 megapixels, at most 200 per document. Local images SHALL resolve only inside the previewed file's root. Remote images SHALL load only with the explicit, default-off preference, over `http`/`https` with URLs of at most 2048 characters and no embedded credentials, no cookies, at most 3 redirects, a 10 second timeout and the byte cap enforced on both `Content-Length` and streamed data. Every resolved address, including after each redirect, SHALL be public; loopback, private, link-local, unique-local, CGNAT, multicast, unspecified and IPv4-mapped equivalents SHALL be refused, and the connection SHALL use the validated address. The preference SHALL hold no secret and a malformed value SHALL mean off.

#### Scenario: Rich Markdown remains inert and bounded
- **WHEN** Markdown contains remote images/links, HTML/script tags, deeply nested
  lists/quotes or excessive table cells and inline nodes
- **THEN** links are not fetched, markup is not executed, remote images are not fetched unless enabled, document depth and
  allocation counts remain bounded, and exceeded caps produce a truncation notice
- **AND** magnifying a preview cannot increase source-read or decoder limits

#### Scenario: Hostile filename to the decoder
- **WHEN** a video file is named `-i evil.mp4` or contains shell metacharacters
- **THEN** the decoder receives it as one argument and no other file or command is touched

#### Scenario: Remote image targets an internal host
- **WHEN** remote images are enabled and an image URL (or a redirect) resolves to `127.0.0.1`, `192.168.x.x`, `169.254.169.254` or `::1`
- **THEN** no request is sent to that address, the placeholder is shown and an error is recorded without any URL credentials

#### Scenario: Oversized or slow remote image
- **WHEN** a remote image declares or streams more than 8 MiB, or stalls beyond 10 seconds
- **THEN** the transfer is aborted and the placeholder is shown without growing memory past the cap
