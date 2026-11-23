//! The Compress dialog (`archive-extract-compress` "Compress to…"): name, format, level,
//! an optional password typed twice, encrypted names (7z) and an optional part size.
//! It only collects input and dispatches `archive.compress`.

use egui::{Context, Id, RichText};
use fm_engine::Session;
use fm_engine::archive_job_cmds::{FORMATS, FormatChoice};
use fm_vfs::VfsUri;
use serde_json::json;

use crate::theme::Tokens;

/// Typed password text: never shown by `Debug`, overwritten when dropped.
#[derive(Clone, Default, PartialEq)]
pub struct Hidden(pub String);

impl std::fmt::Debug for Hidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hidden(<hidden>)")
    }
}

impl Drop for Hidden {
    fn drop(&mut self) {
        let len = self.0.len();
        self.0.clear();
        self.0.extend(std::iter::repeat_n('\0', len));
    }
}

/// The dialog's state.
#[derive(Clone, Debug, PartialEq)]
pub struct CompressForm {
    pub sources: Vec<VfsUri>,
    pub dir: VfsUri,
    /// The name without suffix; the format adds it.
    pub stem: String,
    pub format: usize,
    /// 0 fast, 1 normal, 2 best.
    pub level: usize,
    pub encrypt: bool,
    pub password: Hidden,
    pub confirm: Hidden,
    pub encrypt_names: bool,
    pub split: bool,
    /// Part size in MiB, as typed.
    pub part_mib: String,
}

const LEVELS: [(&str, &str); 3] = [("fast", "Fast"), ("normal", "Normal"), ("best", "Best")];

impl CompressForm {
    /// A form for `sources`, named after the single item or their folder.
    pub fn new(sources: Vec<VfsUri>) -> Option<Self> {
        let target = fm_engine::archive_job_cmds::quick_target(&sources, "")?;
        let dir = target.parent()?;
        let stem = target.file_name()?.to_string();
        Some(Self {
            sources,
            dir,
            stem,
            format: 0,
            level: 1,
            encrypt: false,
            password: Hidden::default(),
            confirm: Hidden::default(),
            encrypt_names: true,
            split: false,
            part_mib: "100".into(),
        })
    }

    fn choice(&self) -> FormatChoice {
        FORMATS.get(self.format).copied().unwrap_or(FORMATS[0])
    }

    fn file_name(&self) -> String {
        format!("{}{}", self.stem.trim(), self.choice().suffix)
    }

    fn part_bytes(&self) -> Result<Option<u64>, String> {
        if !self.split {
            return Ok(None);
        }
        let mib: f64 = self
            .part_mib
            .trim()
            .replace(',', ".")
            .parse()
            .map_err(|_| "Part size must be a number of MiB".to_string())?;
        if !(0.0625..=1_048_576.0).contains(&mib) {
            return Err("Part size must be between 0.0625 and 1048576 MiB".into());
        }
        Ok(Some((mib * 1024.0 * 1024.0) as u64))
    }

    /// What is wrong with the form, if anything; `Ok` holds the command params.
    pub fn params(&self) -> Result<serde_json::Value, String> {
        let name = self.file_name();
        fm_ops_name_check(&name)?;
        let target = self.dir.join(&name).map_err(|e| e.to_string())?;
        let choice = self.choice();
        let password = if self.encrypt && choice.password {
            if self.password.0.is_empty() {
                return Err("Type a password, or turn encryption off".into());
            }
            if self.password.0 != self.confirm.0 {
                return Err("The passwords do not match".into());
            }
            Some(self.password.0.clone())
        } else {
            None
        };
        let level = LEVELS.get(self.level).map_or("normal", |(id, _)| id);
        let sources: Vec<String> = self.sources.iter().map(VfsUri::to_string).collect();
        Ok(json!({
            "sources": sources,
            "target": target.to_string(),
            "format": choice.id,
            "level": level,
            "password": password,
            "encrypt_names": self.encrypt_names,
            "split": self.part_bytes()?,
        }))
    }
}

fn fm_ops_name_check(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.starts_with('.') && name.len() <= 4 {
        return Err("Type a name".into());
    }
    if name.contains(['/', '\\', '\0']) {
        return Err("A name cannot contain / or \\".into());
    }
    Ok(())
}

/// Render the dialog; returns whether it stays open.
pub fn show(
    ctx: &Context,
    s: &mut Session,
    tokens: &Tokens,
    form: &mut CompressForm,
    focus: bool,
    esc: bool,
    enter: bool,
) -> bool {
    let mut open = true;
    let (mut go, mut cancel) = (false, false);
    let verdict = form.params();
    let title = match form.sources.as_slice() {
        [one] => format!("Compress {}", one.file_name().unwrap_or("the selection")),
        many => format!("Compress {} items", many.len()),
    };
    crate::dialog_style::window(ctx, tokens, &title, &mut open)
        .id(Id::NULL.with("fm-compress-dialog"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            egui::Grid::new("fm-compress-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.horizontal(|ui| {
                        let edit =
                            ui.add(egui::TextEdit::singleline(&mut form.stem).desired_width(220.0));
                        if focus {
                            edit.request_focus();
                        }
                        ui.label(RichText::new(form.choice().suffix).color(tokens.text_muted));
                    });
                    ui.end_row();

                    ui.label("Format");
                    egui::ComboBox::from_id_salt("fm-compress-format")
                        .selected_text(form.choice().label)
                        .show_ui(ui, |ui| {
                            for (i, f) in FORMATS.iter().enumerate() {
                                ui.selectable_value(&mut form.format, i, f.label);
                            }
                        });
                    ui.end_row();

                    ui.label("Compression");
                    ui.horizontal(|ui| {
                        for (i, (_, label)) in LEVELS.iter().enumerate() {
                            ui.radio_value(&mut form.level, i, *label);
                        }
                    });
                    ui.end_row();

                    ui.label("Password");
                    ui.add_enabled_ui(form.choice().password, |ui| {
                        ui.checkbox(&mut form.encrypt, "Encrypt with a password")
                            .on_disabled_hover_text(
                                "tar archives cannot be encrypted; choose Zip or 7z",
                            );
                    });
                    ui.end_row();

                    if form.encrypt && form.choice().password {
                        ui.label("");
                        ui.add(
                            egui::TextEdit::singleline(&mut form.password.0)
                                .password(true)
                                .hint_text("Password")
                                .desired_width(220.0),
                        );
                        ui.end_row();
                        ui.label("");
                        ui.add(
                            egui::TextEdit::singleline(&mut form.confirm.0)
                                .password(true)
                                .hint_text("Type it again")
                                .desired_width(220.0),
                        );
                        ui.end_row();
                        if form.choice().id == "7z" {
                            ui.label("");
                            ui.checkbox(&mut form.encrypt_names, "Also hide the file names");
                            ui.end_row();
                        } else {
                            ui.label("");
                            ui.label(
                                RichText::new(
                                    "Zip encrypts contents with AES-256; names stay visible.",
                                )
                                .color(tokens.text_muted),
                            );
                            ui.end_row();
                        }
                    }

                    ui.label("Split");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut form.split, "Into parts of");
                        ui.add_enabled(
                            form.split,
                            egui::TextEdit::singleline(&mut form.part_mib).desired_width(70.0),
                        );
                        ui.label("MiB");
                    });
                    ui.end_row();
                });
            if let Err(message) = &verdict {
                ui.colored_label(tokens.error, message);
            }
            ui.separator();
            ui.horizontal(|ui| {
                go = ui
                    .add_enabled(verdict.is_ok(), egui::Button::new("Compress"))
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if !open || cancel || esc {
        return false;
    }
    if (go || enter)
        && let Ok(params) = verdict
    {
        s.dispatch_or_notice("archive.compress", &params);
        return false;
    }
    true
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn form() -> CompressForm {
        let a = VfsUri::parse("/tmp/photos/a.jpg").expect("a");
        let b = VfsUri::parse("/tmp/photos/b.jpg").expect("b");
        CompressForm::new(vec![a, b]).expect("form")
    }

    #[test]
    fn several_items_are_named_after_their_folder() {
        let f = form();
        assert_eq!(f.stem, "photos");
        let p = f.params().expect("ok");
        assert_eq!(p["target"], "file:///tmp/photos/photos.zip");
        assert_eq!(p["password"], serde_json::Value::Null);
    }

    #[test]
    fn passwords_must_match_and_tar_ignores_them() {
        let mut f = form();
        f.encrypt = true;
        assert!(f.params().is_err());
        f.password.0 = "secret".into();
        f.confirm.0 = "other".into();
        assert!(f.params().is_err());
        f.confirm.0 = "secret".into();
        assert_eq!(f.params().expect("ok")["password"], "secret");
        assert!(!format!("{f:?}").contains("secret"));
        f.format = 2;
        let p = f.params().expect("tar");
        assert_eq!(p["password"], serde_json::Value::Null);
        assert_eq!(p["target"], "file:///tmp/photos/photos.tar.gz");
    }

    #[test]
    fn part_size_and_name_are_checked() {
        let mut f = form();
        f.split = true;
        f.part_mib = "abc".into();
        assert!(f.params().is_err());
        f.part_mib = "1,5".into();
        assert_eq!(f.params().expect("ok")["split"], 1_572_864);
        f.stem = "a/b".into();
        assert!(f.params().is_err());
        f.stem = "  ".into();
        assert!(f.params().is_err());
    }
}
