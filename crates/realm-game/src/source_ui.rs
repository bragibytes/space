//! Settings UI for choosing the colony source directory.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::colony_dir::{default_source_dir, expand_path, save_source_dir};
use crate::{attach_source_dir, ColonyCode};

#[derive(Resource)]
pub struct SourceSettingsUi {
    pub open: bool,
    pub draft_path: String,
    pub message: String,
}

impl Default for SourceSettingsUi {
    fn default() -> Self {
        Self {
            open: false,
            draft_path: String::new(),
            message: String::new(),
        }
    }
}

pub fn toggle_source_settings(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<SourceSettingsUi>,
) {
    if keyboard.just_pressed(KeyCode::F2) {
        ui.open = !ui.open;
    }
}

pub fn draw_source_settings(
    mut contexts: EguiContexts,
    mut settings: ResMut<SourceSettingsUi>,
    mut code: ResMut<ColonyCode>,
) {
    let default_label = default_source_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.creeps/colony".into());

    egui::TopBottomPanel::top("source_bar").show(contexts.ctx_mut().unwrap(), |ui| {
        ui.horizontal(|ui| {
            ui.strong("Source:");
            ui.label(&code.colony_path);
            if ui.button("Change…").clicked() {
                settings.draft_path = code.colony_path.clone();
                settings.open = true;
            }
        });
    });

    if !settings.open {
        return;
    }

    let mut apply = false;
    let mut reset = false;

    egui::Window::new("Colony source directory")
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .show(contexts.ctx_mut().unwrap(), |ui| {
            ui.label("Point the game at your Rust colony project. Saves auto-rebuild and reload.");
            ui.separator();

            ui.label("Directory path:");
            ui.add(
                egui::TextEdit::singleline(&mut settings.draft_path)
                    .desired_width(f32::INFINITY)
                    .hint_text(&default_label),
            );

            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                if ui.button(format!("Reset to {default_label}")).clicked() {
                    reset = true;
                }
                if ui.button("Close").clicked() {
                    settings.open = false;
                }
            });

            if !settings.message.is_empty() {
                ui.separator();
                ui.colored_label(egui::Color32::LIGHT_GREEN, &settings.message);
            }

            ui.separator();
            ui.label("Tips:");
            ui.label("• Default: ~/.creeps/colony/ — or any path you prefer");
            ui.label("• Folder needs Cargo.toml (created automatically if missing)");
            ui.label("• Press F2 to open this panel");
            ui.label("• Env override: CREEPS_COLONY_DIR");
        });

    if reset {
        settings.draft_path = default_label.clone();
    }

    if apply || reset {
        let path = expand_path(&settings.draft_path);
        match attach_source_dir(&mut code, path.clone()) {
            Ok(()) => {
                if let Err(err) = save_source_dir(&path) {
                    settings.message = format!("Watching {path:?} (could not save preference: {err})");
                } else {
                    settings.message = format!("Now watching {}", path.display());
                }
                settings.open = false;
            }
            Err(err) => {
                settings.message = err.to_string();
            }
        }
    }
}

pub fn init_source_settings(mut settings: ResMut<SourceSettingsUi>, code: Res<ColonyCode>) {
    if settings.draft_path.is_empty() {
        settings.draft_path = code.colony_path.clone();
    }
}