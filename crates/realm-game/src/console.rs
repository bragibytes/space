//! In-game console for one-off commands (Screeps-style), alongside file-based AI.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

use crate::colony_dir::{expand_path, save_source_dir};
use crate::{attach_source_dir, ColonyCode, SimResource};

const MAX_LOG_LINES: usize = 200;

#[derive(Resource)]
pub struct GameConsole {
    pub open: bool,
    pub input: String,
    pub log: Vec<String>,
    pub paused: bool,
    pub pending_manual_tick: bool,
}

impl Default for GameConsole {
    fn default() -> Self {
        Self {
            open: false,
            input: String::new(),
            log: vec![
                "Creeps console — type `help` for commands.".into(),
                "Your main AI runs from ~/.creeps/colony/ (F2 to change directory).".into(),
            ],
            paused: false,
            pending_manual_tick: false,
        }
    }
}

pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameConsole>()
            .add_plugins(EguiPlugin::default())
            .add_systems(PreUpdate, toggle_console)
            .add_systems(EguiPrimaryContextPass, draw_console);
    }
}

fn toggle_console(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut console: ResMut<GameConsole>,
) {
    if keyboard.just_pressed(KeyCode::Backquote) || keyboard.just_pressed(KeyCode::F1) {
        console.open = !console.open;
    }
}

fn draw_console(
    mut contexts: EguiContexts,
    mut console: ResMut<GameConsole>,
    sim: Res<SimResource>,
    mut code: ResMut<ColonyCode>,
) {
    if !console.open {
        return;
    }

    let mut submit = false;
    egui::Window::new("Console")
        .default_width(520.0)
        .default_height(280.0)
        .resizable(true)
        .show(contexts.ctx_mut().unwrap(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &console.log {
                        ui.label(line);
                    }
                });

            ui.separator();

            ui.horizontal(|ui| {
                ui.label(">");
                ui.add(
                    egui::TextEdit::singleline(&mut console.input)
                        .desired_width(f32::INFINITY)
                        .hint_text("help | status | creeps | source <path> | pause"),
                );
            });

            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !console.input.trim().is_empty() {
                submit = true;
            }
        });

    if submit && !console.input.trim().is_empty() {
        let line = console.input.trim().to_string();
        console.input.clear();
        console_push(&mut console, format!("> {line}"));
        execute_command(&line, &mut console, &sim, &mut code);
    }
}

fn console_push(console: &mut GameConsole, line: impl Into<String>) {
    console.log.push(line.into());
    if console.log.len() > MAX_LOG_LINES {
        let drain = console.log.len() - MAX_LOG_LINES;
        console.log.drain(0..drain);
    }
}

fn execute_command(
    line: &str,
    console: &mut GameConsole,
    sim: &SimResource,
    code: &mut ColonyCode,
) {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("").to_lowercase();

    match cmd.as_str() {
        "help" | "?" => {
            console_push(
                console,
                "Commands: help, status, creeps, room, path, source <dir>, pause, resume, tick, clear",
            );
            console_push(
                console,
                "Main loop: edit src/lib.rs in your source directory (F2 to change path).",
            );
            console_push(
                console,
                "Console: one-off inspection and control. `eval` (script snippets) coming soon.",
            );
        }
        "clear" => console.log.clear(),
        "pause" => {
            console.paused = true;
            console_push(console, "Simulation paused.");
        }
        "resume" | "unpause" => {
            console.paused = false;
            console_push(console, "Simulation resumed.");
        }
        "tick" => {
            console_push(console, format!("Manual tick at {}", sim.snapshot.tick));
            // actual tick triggered via flag — handled in main
            console.pending_manual_tick = true;
        }
        "status" => {
            console_push(
                console,
                format!(
                    "tick {} | room {} | code: {} | {}",
                    sim.snapshot.tick,
                    sim.active_room,
                    code.status,
                    if code.runner.is_some() {
                        "wasm loaded"
                    } else {
                        "built-in AI"
                    }
                ),
            );
        }
        "path" => console_push(console, format!("Source directory: {}", code.colony_path)),
        "source" | "setsource" => {
            let rest: String = parts.collect::<Vec<_>>().join(" ");
            if rest.is_empty() {
                console_push(console, "Usage: source <path>  e.g. source ~/.creeps/colony");
            } else {
                let path = expand_path(&rest);
                match attach_source_dir(code, path.clone()) {
                    Ok(()) => {
                        if let Err(err) = save_source_dir(&path) {
                            console_push(
                                console,
                                format!("Watching {} (could not save preference: {err})", path.display()),
                            );
                        } else {
                            console_push(console, format!("Now watching {}", path.display()));
                        }
                    }
                    Err(err) => console_push(console, format!("Error: {err}")),
                }
            }
        }
        "room" => {
            let room = sim
                .snapshot
                .rooms
                .iter()
                .find(|r| r.name == sim.active_room);
            if let Some(room) = room {
                console_push(
                    console,
                    format!(
                        "{} — {} structures, {} creeps",
                        room.name,
                        room.structures.len(),
                        room.creeps.len()
                    ),
                );
            }
        }
        "creeps" => {
            let room = sim
                .snapshot
                .rooms
                .iter()
                .find(|r| r.name == sim.active_room);
            if let Some(room) = room {
                if room.creeps.is_empty() {
                    console_push(console, "No creeps in this room.");
                }
                for c in &room.creeps {
                    console_push(
                        console,
                        format!(
                            "  {} ({}) @ {},{} | {} | carry {}/{}",
                            c.name,
                            c.owner,
                            c.x,
                            c.y,
                            c.action,
                            c.carrying_energy,
                            c.carrying_capacity
                        ),
                    );
                }
            }
        }
        "eval" => {
            let rest: String = parts.collect::<Vec<_>>().join(" ");
            if rest.is_empty() {
                console_push(console, "Usage: eval <snippet> — not wired yet.");
            } else {
                console_push(
                    console,
                    "eval not implemented yet — use your colony lib.rs for now.",
                );
            }
        }
        "" => {}
        other => console_push(console, format!("Unknown command: {other}. Type `help`.")),
    }
}