## ADDED Requirements

### Requirement: Subtitle sources and selection
The pane SHALL list embedded subtitle tracks and external subtitle files next to the video with the same base name (`.srt`, `.vtt`, `.ass`, `.ssa`, also with a language suffix such as `.en.srt`), SHALL let the user pick one or none, and SHALL remember the preferred language per session. Subtitles SHALL be off when none matches the preferred language. `media.subtitle.load` SHALL add an external file chosen by the user.

#### Scenario: External file next to the video
- **WHEN** `movie.mkv` plays and `movie.en.srt` is in the same folder
- **THEN** the subtitle list offers it and it can be selected

#### Scenario: Off by default without a match
- **WHEN** no subtitle track matches the preferred language
- **THEN** no subtitles are shown and the list still allows choosing one

### Requirement: Text subtitles
SRT and WebVTT cues (and embedded text tracks) SHALL be parsed and drawn by the UI over the video with readable size, outline and a theme-aware colour; size and vertical position SHALL be adjustable and persist. ASS/SSA tracks SHALL be rendered with their styling by the decoder (burned into the frames). Subtitle delay SHALL be adjustable in 100 ms steps with `media.subtitle.delay`.

#### Scenario: Delay adjustment
- **WHEN** the delay is set to +500 ms
- **THEN** each cue appears 500 ms later than its timestamp

#### Scenario: Malformed file
- **WHEN** a subtitle file is not valid text, is larger than the cap or has no cues
- **THEN** it is rejected with an error and playback continues without it

### Requirement: Bitmap subtitles
Bitmap subtitle tracks (PGS and VobSub/DVD) SHALL be rendered by the decoder into the video frames, so they appear without a separate decoding path in the UI. Switching a bitmap or ASS track SHALL restart the video decoder at the current position without interrupting audio.

#### Scenario: PGS track
- **WHEN** a Blu-ray rip with a PGS track is played with that track selected
- **THEN** the subtitles appear over the picture in time with the dialogue

#### Scenario: Switch while playing
- **WHEN** the subtitle track is switched during playback
- **THEN** audio continues without a gap and the picture resumes with the new subtitles within a second
