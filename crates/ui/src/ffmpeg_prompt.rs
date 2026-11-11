//! Offered when a video cannot play because ffmpeg is missing: download it, or install it by hand.

use egui::{Context, RichText};
use fm_engine::{Params, Session};

use crate::dialog_style::{apply, hint, window};
use crate::theme::Tokens;

const WINDOWS_BUILDS: &str = "https://www.gyan.dev/ffmpeg/builds/";
const DOWNLOAD_PAGE: &str = "https://ffmpeg.org/download.html";

pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    if !s.ffmpeg.prompt {
        return;
    }
    let found = fm_preview::video::find_tool("ffmpeg").is_some()
        && fm_preview::video::find_tool("ffprobe").is_some();
    if found && !s.ffmpeg.installing() {
        s.ffmpeg.prompt = false;
        return;
    }
    let mut open = true;
    window(ctx, tokens, "ffmpeg is needed to play video", &mut open).show(ctx, |ui| {
        apply(ui, tokens);
        ui.set_max_width(420.0);
        ui.label("Video playback uses ffmpeg and ffprobe, which are not installed.");
        if s.ffmpeg.installing() {
            let (done, total) = s.ffmpeg.progress();
            let fraction = if total == 0 {
                0.0
            } else {
                (done as f64 / total as f64) as f32
            };
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(380.0)
                    .text(format!(
                        "Downloading… {:.0} of {:.0} MB",
                        done as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    )),
            );
        } else if s.ffmpeg.downloadable() {
            ui.label(RichText::new("Download automatically").strong());
            hint(
                ui,
                tokens,
                "The app downloads a pinned build over https and verifies its checksum before installing it.",
            );
            if ui.button("Download ffmpeg").clicked() {
                s.dispatch_or_notice("tools.ffmpeg.install", &Params::Null);
            }
        }
        if let Some(error) = s.ffmpeg.error.clone() {
            ui.label(RichText::new(error).color(tokens.error));
        }
        ui.separator();
        ui.label(RichText::new("Install it yourself").strong());
        if cfg!(windows) {
            ui.horizontal_wrapped(|ui| {
                ui.label("Download a build from");
                ui.hyperlink_to(WINDOWS_BUILDS, WINDOWS_BUILDS);
            });
            hint(
                ui,
                tokens,
                "Put ffmpeg.exe and ffprobe.exe in a folder on your PATH, then play the video again.",
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label("Install it with your package manager (for example `brew install ffmpeg` or `apt install ffmpeg`), or see");
                ui.hyperlink_to(DOWNLOAD_PAGE, DOWNLOAD_PAGE);
            });
            hint(ui, tokens, "Then play the video again.");
        }
    });
    if !open {
        s.ffmpeg.prompt = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_renders_and_can_be_dismissed_without_a_download() {
        let ctx = Context::default();
        let mut s = Session::default();
        s.ffmpeg.prompt = true;
        let tokens = Tokens::dark();
        let mut output = ctx.run_ui(Default::default(), |ctx| show(ctx, &mut s, &tokens));
        output.textures_delta.clear();
        assert!(!s.ffmpeg.installing());
    }
}
