## ADDED Requirements

### Requirement: Audio pipeline
All audio, whichever decoder produced it, SHALL pass through one pipeline in `fm-media`: decode to 32-bit float PCM, a single high-quality resample to the output device's native rate and channel layout (never more than one resample), preamp, equalizer, volume (with a soft limiter against clipping) and the output sink. Output SHALL use the platform's native API (WASAPI, CoreAudio, ALSA directly or through PipeWire/PulseAudio compatibility) and SHALL follow default-device changes. A missing or failing device SHALL produce an actionable error and not crash.

Audio files that the in-process decoder (Symphonia) cannot read, such as WMA, Opus, APE, WavPack, AIFF, AC-3 and DTS, SHALL be decoded by an `ffmpeg` subprocess into the same pipeline, in the Media pane and in Preview alike, with tags and duration from `ffprobe`, seeking by restarting the decoder, a bounded read-ahead buffer, and a timeout that reports a stalled decoder as an error. Without ffmpeg such files SHALL report that the format needs ffmpeg.

#### Scenario: WMA file
- **WHEN** the user plays a `.wma` file
- **THEN** it plays, seeks and shows its duration like an MP3

#### Scenario: Different sample rates
- **WHEN** a 44.1 kHz and a 96 kHz file play on a 48 kHz device
- **THEN** each is resampled once to 48 kHz and neither stutters or changes pitch

#### Scenario: Device removed
- **WHEN** the output device disappears during playback
- **THEN** playback pauses with a notice, and resumes on the new default device when the user plays

### Requirement: Gapless playback
Consecutive items decoded in-process (mp3, flac, ogg/vorbis, wav, aac/m4a, alac) SHALL play without an audible gap: the next item is opened ahead of the end, encoder delay and padding (LAME/Xing, iTunSMPB, Vorbis, FLAC) are trimmed, and the sink stays open. Items decoded by ffmpeg MAY have a short gap. Gapless SHALL be a preference, on by default.

#### Scenario: Album of live tracks
- **WHEN** two consecutive FLAC tracks that continue each other play with gapless on
- **THEN** no silence is inserted at the boundary

### Requirement: Equalizer
The pane SHALL provide a 10-band graphic equalizer (31 Hz–16 kHz, ±12 dB) with a preamp, built-in presets (flat, bass boost, treble boost, vocal, loudness, and others), user-saved presets, and an on/off switch. It SHALL apply to audio and to video soundtracks, change without interrupting playback, and persist. The preamp SHALL default to a level that avoids clipping when bands are boosted.

#### Scenario: Boost bass while playing
- **WHEN** the 62 Hz band is raised during playback
- **THEN** the change is audible within a fraction of a second with no dropout, and persists after restart

#### Scenario: Bad preset values
- **WHEN** a preset contains a gain outside the range or a wrong number of bands
- **THEN** it is rejected with an error and the current equalizer is unchanged

### Requirement: Playback speed with pitch kept
`media.speed` SHALL accept 0.5× to 3× in steps no larger than 0.05× and SHALL keep pitch unchanged for audio and video soundtracks; video frames follow the audio clock. Audio speed SHALL work in-process for every format the app decodes, without ffmpeg. Speed SHALL persist per audiobook and otherwise per session. The reported position SHALL stay in media time, so the clock, seek bar, resume points and chapters are unaffected by speed.

#### Scenario: Speed up
- **WHEN** speed is set to 1.5× during playback
- **THEN** playback is 1.5× faster with the same pitch and no gap larger than 300 ms

#### Scenario: Out of range
- **WHEN** `media.speed` is given 0, 5 or NaN
- **THEN** it returns an error and the speed is unchanged

#### Scenario: Audiobook keeps its speed
- **WHEN** an audiobook is set to 1.5×, a song is then played at the session speed 1.25×, and the book is played again (also after a restart)
- **THEN** the book plays at 1.5× and the song at 1.25×

#### Scenario: Speed without ffmpeg
- **WHEN** ffmpeg is not installed and an mp3 is set to 2×
- **THEN** it plays at 2× with the same pitch, and a 60 s file finishes after about 30 s with the clock showing 1:00

### Requirement: Loop and sleep timer
`media.loop` SHALL set loop points A and B on the current item (A at the current position, then B), clear them, and SHALL loop between them until cleared or the item changes. `media.sleep` SHALL stop playback after a chosen number of minutes, at the end of the current item, or at the end of the current chapter, with a 10 s fade-out, and SHALL be cancellable and visible in the pane while armed.

#### Scenario: A-B loop
- **WHEN** A is set at 1:00 and B at 1:30
- **THEN** playback returns to 1:00 whenever it reaches 1:30 until the loop is cleared

#### Scenario: Sleep timer
- **WHEN** the timer is set to 30 minutes
- **THEN** playback fades out and stops after 30 minutes, and the pane no longer shows an armed timer
