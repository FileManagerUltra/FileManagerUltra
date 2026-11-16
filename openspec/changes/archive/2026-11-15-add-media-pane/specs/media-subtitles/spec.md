## ADDED Requirements

### Requirement: Subtitle sources and selection
The pane SHALL list embedded subtitle tracks and external subtitle files next to a local video with the same base name (`.srt`, `.vtt`, `.ass`, `.ssa`, also with tags such as `.en.srt` or `.en.forced.srt`; at most 32, symlinks not followed), SHALL let the user pick one or none, and SHALL remember the language of the last explicit choice for the rest of the app session (turning subtitles off clears it). Subtitles SHALL be off when none matches the preferred language. `media.subtitle.load` SHALL add an external file chosen by the user.

#### Scenario: External file next to the video
- **WHEN** `movie.mkv` plays and `movie.en.srt` is in the same folder
- **THEN** the subtitle list offers it and it can be selected

#### Scenario: Off by default without a match
- **WHEN** no subtitle track matches the preferred language
- **THEN** no subtitles are shown and the list still allows choosing one

### Requirement: Text subtitles
SRT and WebVTT files, ASS/SSA files and embedded text tracks (SubRip, ASS/SSA, WebVTT, mov_text and other text codecs) SHALL be parsed into timed cues off the UI thread and drawn by the UI over the video, in the Media pane and in fullscreen. Markup and ASS override tags SHALL be removed so every text format is drawn with the same user-chosen appearance; styling inside ASS/SSA files SHALL NOT be used. Subtitle delay SHALL be adjustable in 100 ms steps with `media.subtitle.delay` (also G/H keys), within ±10 minutes, and SHALL reset for the next item. Subtitle files SHALL be capped (8 MiB, 100 000 cues, 8 lines and 512 characters per line); UTF-8, UTF-16 with a byte-order mark and single-byte legacy encodings SHALL be accepted, binary data refused.

#### Scenario: Delay adjustment
- **WHEN** the delay is set to +500 ms
- **THEN** each cue appears 500 ms later than its timestamp

#### Scenario: Malformed file
- **WHEN** a subtitle file is not valid text, is larger than the cap or has no cues
- **THEN** it is rejected with an error and playback continues without it

#### Scenario: Same look for every format
- **WHEN** an SRT file, an embedded text track and an ASS file are shown in turn with the same settings
- **THEN** all three use the same font, size, colours, background, edge and position

### Requirement: Subtitle appearance for readability
The Media page of Preferences SHALL have a Subtitles section whose settings apply to every text subtitle type: size (50-400 %), distance from the bottom of the picture, font (sans-serif or monospace), bold, text colour and opacity (at least 25 %), a background box with colour and opacity, an edge style (none, outline, drop shadow, raised, depressed) with colour and thickness, letter spacing and line spacing. It SHALL offer presets for common low-vision needs (high contrast white on black, yellow on black, black on white, large text with a heavy outline), a reset, a live preview over a dark and a bright sample scene drawn exactly as on the video, and the resulting WCAG contrast ratio with a warning below 4.5:1. Settings SHALL be changed through `media.subtitle.style` (all-or-nothing validation), persist in the preferences file, and out-of-range hand edits SHALL be clamped. Default colours SHALL come from theme tokens, and the section SHALL render in dark and light themes.

#### Scenario: High-contrast preset
- **WHEN** the user picks "Yellow on black"
- **THEN** subtitles show in yellow on an opaque black box and the preview reports a contrast of at least 7:1

#### Scenario: Low contrast warning
- **WHEN** the user chooses grey text with no background and no edge
- **THEN** the preview shows the contrast ratio with a warning that it is hard to read

#### Scenario: Invalid value
- **WHEN** `media.subtitle.style` receives an unknown field, a colour that is not `#rrggbb` or a value out of range
- **THEN** it returns an error and no setting changes

### Requirement: Bitmap subtitles
Bitmap subtitle tracks (PGS and VobSub/DVD) SHALL be selectable and shown over the video, drawn into the frames by the video decoder. They SHALL honour the subtitle size setting (scaled from the bottom centre, capped at 0.5×–1.5× of the authored size) and the distance from bottom (raised by the amount above the default; lower values keep the authored place). Their colours and background are part of the picture and cannot be changed, and the Subtitles section SHALL say so. A subtitle already on screen at a seek point SHALL be shown (the decoder starts up to 2 s early). Switching a bitmap track, or changing its size or distance while it plays, SHALL restart only the video decoder at the current position without interrupting audio.

#### Scenario: PGS track
- **WHEN** a Blu-ray rip with a PGS track is played with that track selected
- **THEN** the subtitles appear over the picture in time with the dialogue

#### Scenario: Bigger picture subtitles
- **WHEN** the subtitle size is 120% and the distance from bottom is 25% while a PGS track is shown
- **THEN** the subtitle picture is 1.2× its authored size and raised by 20% of the picture height

#### Scenario: Switch while playing
- **WHEN** the subtitle track is switched during playback
- **THEN** audio continues without a gap and the picture resumes with the new subtitles within a second
