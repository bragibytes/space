use realm_core::types::{ClassStats, CLASSES};
use realm_protocol::ClassName;

/// Unicode progress bar for terminal UIs.
pub fn meter(current: i32, max: i32, width: usize) -> String {
    if max <= 0 {
        return "░".repeat(width);
    }
    let filled = ((current as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

pub fn class_select_text() -> String {
    let mut lines = vec![
        "Choose your class:".into(),
        String::new(),
        "warrior — heavy fighter".into(),
        "mage    — arcane caster".into(),
        "rogue   — swift striker".into(),
    ];
    for cls in [ClassName::Warrior, ClassName::Mage, ClassName::Rogue] {
        if let Some(stats) = CLASSES.get(&cls) {
            lines.push(String::new());
            lines.extend(class_blurb(stats));
        }
    }
    lines.join("\n")
}

fn class_blurb(cls: &ClassStats) -> Vec<String> {
    let mut lines = vec![
        format!("{} — {}", cls.name.as_str(), cls.display_name),
        cls.description.to_string(),
    ];
    for row in cls.art {
        lines.push(format!("  {row}"));
    }
    lines.push(format!(
        "  HP {}  MP {}  ATK {}",
        cls.max_hp, cls.max_mp, cls.attack
    ));
    lines
}

pub fn combat_target_line(
    target: &str,
    hp: i32,
    max_hp: i32,
) -> String {
    format!("⚔ {target}  {} {hp}/{max_hp}", meter(hp, max_hp, 10))
}