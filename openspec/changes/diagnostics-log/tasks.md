## 1. Implementation

- [x] 1.1 `fm-engine::diag`: atomic switch, bounded channel, writer thread (file, size cap, rate limit, per-second process/GPU memory sample with sysinfo), counters, `log` sink with target allow-list and a bounded startup buffer, old-log pruning
- [x] 1.2 Prefs `diagnostics_log` (off) and `video_texture_reuse` (on) through `prefs.set`; `app.diagnostics.folder`; closed-schema test updated
- [x] 1.3 Hooks: `Session::dispatch` (command id), video pump (decoder start, frames received), `fm-preview::player::live_players()`
- [x] 1.4 `fm-ui`: video texture with grow-only storage and UV sub-rect behind the switch, upload counters, frame-cache eviction counters and grace period
- [x] 1.5 App: start/stop with the preference, startup summary, once-per-second UI sample (textures, frames, panes), build commit in About
- [x] 1.6 Preferences › Advanced › Troubleshooting section
- [x] 1.7 Tests: log off creates nothing; privacy (paths in params never logged); size cap and rate limit; malformed params; texture reuse across sizes and missed frames; preferences round-trip
- [x] 1.8 CHANGELOG, devlog
- [x] 1.10 Round 2 (after the first Windows logs): texture exactly frame-sized (grow-only sub-rect crept ~7 MB/min on NVIDIA OpenGL); counters for live frames and bytes, helper processes, thumbnails made/from disk/pending, preview renders, hidden Media playback
- [ ] 1.9 Manual (Windows): play 1080p video with the log on, reuse on and off; compare memory columns
