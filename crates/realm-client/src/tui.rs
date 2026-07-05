use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use ratatui::Terminal;
use realm_core::minimap::render_minimap_ascii;
use realm_protocol::{ClassName, MinimapCell, OnlinePlayer, OutputStyle, PlayerSnapshot, ServerMessage};
use tokio::sync::mpsc;

use crate::app::{
    handle_user_input, reconnect_delay, run_connection, should_reconnect,
    AuthStep, ClientState, WsEvent,
};
use crate::ui::{class_select_text, combat_target_line, meter};

struct RoomView {
    title: String,
    description: String,
    exits: String,
    entities: Vec<String>,
    zone_art: Option<String>,
    minimap: Option<Vec<MinimapCell>>,
}

struct TuiState {
    client: ClientState,
    log_lines: Vec<(String, OutputStyle)>,
    input: String,
    stats: Option<PlayerSnapshot>,
    online: Vec<OnlinePlayer>,
    room: Option<RoomView>,
    ticker: String,
    flash_until: Option<Instant>,
    flash_color: Color,
    in_combat: bool,
    combat_target: Option<String>,
    combat_target_hp: Option<i32>,
    combat_target_max_hp: Option<i32>,
    status_message: String,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            client: ClientState::default(),
            log_lines: Vec::new(),
            input: String::new(),
            stats: None,
            online: Vec::new(),
            room: None,
            ticker: String::new(),
            flash_until: None,
            flash_color: Color::Cyan,
            in_combat: false,
            combat_target: None,
            combat_target_hp: None,
            combat_target_max_hp: None,
            status_message: String::new(),
        }
    }
}

pub async fn run(server_url: &str) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(server_url, &mut terminal).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

async fn run_tui_loop(
    server_url: &str,
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    let mut state = TuiState::default();

    loop {
        let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<WsEvent>();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let url = server_url.to_string();
        let conn_state = state.client.clone();
        let conn_task = tokio::spawn(async move {
            let _ = run_connection(&url, &conn_state, ws_tx, cmd_rx).await;
        });

        let mut connected = false;
        let mut needs_render = true;

        'session: loop {
            if needs_render {
                terminal.draw(|f| render_ui(f, &state))?;
                needs_render = false;
            }

            if state.flash_until.is_some_and(|t| Instant::now() > t) {
                state.flash_until = None;
                needs_render = true;
            }

            let tick = tokio::time::sleep(Duration::from_millis(50));
            tokio::pin!(tick);

            tokio::select! {
                event = ws_rx.recv() => {
                    let Some(event) = event else { break 'session };
                    match event {
                        WsEvent::Connected => {
                            connected = true;
                            state.client.reconnect_attempts = 0;
                        }
                        WsEvent::Message(msg) => {
                            apply_message(&mut state, msg);
                            needs_render = true;
                        }
                        WsEvent::Error(err) => {
                            if !connected && state.client.reconnect_attempts == 0 && !state.client.authenticated {
                                state.status_message = format!("Connection error: {err}");
                                needs_render = true;
                                conn_task.abort();
                                return Ok(());
                            }
                        }
                        WsEvent::Disconnected => {
                            if state.client.intentional_disconnect {
                                state.status_message = "Farewell, adventurer!".into();
                                needs_render = true;
                                conn_task.abort();
                                return Ok(());
                            }
                            if should_reconnect(&state.client) {
                                state.client.reconnect_attempts += 1;
                                state.status_message = format!(
                                    "Reconnecting ({}/5)...",
                                    state.client.reconnect_attempts
                                );
                                needs_render = true;
                                break 'session;
                            }
                            state.status_message = "Disconnected from server.".into();
                            needs_render = true;
                            conn_task.abort();
                            return Ok(());
                        }
                    }
                }
                _ = tick => {
                    if event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key) = event::read()? {
                            if handle_key(&mut state, key, &cmd_tx) {
                                conn_task.abort();
                                return Ok(());
                            }
                            needs_render = true;
                        }
                    }
                }
            }
        }

        conn_task.abort();
        reconnect_delay(state.client.reconnect_attempts).await;
    }
}

fn handle_key(
    state: &mut TuiState,
    key: KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<realm_protocol::ClientMessage>,
) -> bool {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Enter => {
            let line = state.input.clone();
            state.input.clear();
            if let Some(msg) = handle_user_input(&mut state.client, &line) {
                let _ = cmd_tx.send(msg);
            } else {
                match state.client.auth_step {
                    AuthStep::Mode if line.eq_ignore_ascii_case("login") => {
                        append_log(state, "Enter your username.".into(), OutputStyle::System);
                    }
                    AuthStep::Mode if line.eq_ignore_ascii_case("register") => {
                        append_log(state, "Choose a username (3-16 chars).".into(), OutputStyle::System);
                    }
                    AuthStep::Mode if !line.is_empty() => {
                        append_log(state, "Type \"login\" or \"register\".".into(), OutputStyle::System);
                    }
                    AuthStep::Password if state.client.pending_password != "__register__" => {
                        append_log(state, "Enter password.".into(), OutputStyle::System);
                    }
                    AuthStep::Class if !line.is_empty() => {
                        append_log(
                            state,
                            "Choose: warrior, mage, or rogue".into(),
                            OutputStyle::System,
                        );
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Backspace => {
            state.input.pop();
        }
        KeyCode::Char(c) => {
            if !state.client.hidden_input {
                state.input.push(c);
            } else {
                state.input.push(c);
            }
        }
        _ => {}
    }
    false
}

fn apply_message(state: &mut TuiState, msg: ServerMessage) {
    match msg {
        ServerMessage::Banner => {
            append_log(state, "REALM OF ECHOES".into(), OutputStyle::System);
            append_log(state, "The realm awaits...".into(), OutputStyle::System);
        }
        ServerMessage::Output { text, style } => {
            let style = style.unwrap_or(OutputStyle::Normal);
            for line in text.lines() {
                let prefix = if style == OutputStyle::Combat { "⚔ " } else { "" };
                append_log(state, format!("{prefix}{line}"), style);
            }
        }
        ServerMessage::Room {
            title,
            description,
            exits,
            entities,
            zone_art,
            minimap,
            ..
        } => {
            state.room = Some(RoomView {
                title: title.clone(),
                description,
                exits,
                entities,
                zone_art,
                minimap,
            });
            append_log(state, format!("▣ {title}"), OutputStyle::Normal);
        }
        ServerMessage::Stats { player } => {
            state.in_combat = player.in_combat.unwrap_or(false);
            state.stats = Some(player);
        }
        ServerMessage::Online { players } => {
            state.online = players;
        }
        ServerMessage::Flash { color } => {
            state.flash_color = match color.as_str() {
                "red" => Color::Red,
                "yellow" => Color::Yellow,
                "green" => Color::Green,
                _ => Color::Cyan,
            };
            state.flash_until = Some(Instant::now() + Duration::from_millis(120));
        }
        ServerMessage::Bell => {
            let _ = crossterm::execute!(io::stdout(), crossterm::style::Print("\x07"));
        }
        ServerMessage::Ticker { text } => {
            state.ticker = text;
        }
        ServerMessage::Motd { text } => {
            append_log(state, "=== Message of the Day ===".into(), OutputStyle::System);
            append_log(state, text, OutputStyle::System);
        }
        ServerMessage::Error { text } => {
            append_log(state, format!("✗ {text}"), OutputStyle::System);
        }
        ServerMessage::Prompt { text } => {
            if state.client.authenticated {
                state.client.prompt = ">".into();
            } else {
                state.client.prompt = text.trim().to_string();
                if state.client.prompt.is_empty() {
                    state.client.prompt = "login or register?".into();
                }
            }
            if state.client.auth_step == AuthStep::Class {
                state.status_message = "Choose class: warrior | mage | rogue".into();
            }
        }
        ServerMessage::Disconnect { reason } => {
            state.client.intentional_disconnect = true;
            append_log(state, reason, OutputStyle::System);
        }
        ServerMessage::Combat { state: combat } => {
            state.in_combat = combat.in_combat;
            if combat.in_combat {
                state.combat_target = combat.target.clone();
                state.combat_target_hp = combat.target_hp;
                state.combat_target_max_hp = combat.target_max_hp;
            } else {
                state.combat_target = None;
                state.combat_target_hp = None;
                state.combat_target_max_hp = None;
            }
        }
    }
}

fn append_log(state: &mut TuiState, text: String, style: OutputStyle) {
    state.log_lines.push((text, style));
    if state.log_lines.len() > 500 {
        let drain = state.log_lines.len() - 500;
        state.log_lines.drain(0..drain);
    }
}

fn render_ui(f: &mut Frame, state: &TuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    render_header(f, chunks[0], state);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
        .split(chunks[1]);
    render_log(f, body[0], state);
    render_sidebar(f, body[1], state);
    render_input(f, chunks[2], state);
}

fn render_header(f: &mut Frame, area: Rect, state: &TuiState) {
    let border_color = if state.flash_until.is_some() {
        state.flash_color
    } else if state.in_combat {
        Color::Red
    } else {
        Color::Cyan
    };

    let content = if let Some(p) = &state.stats {
        let cls = match p.class_name {
            ClassName::Warrior => "Warrior",
            ClassName::Mage => "Mage",
            ClassName::Rogue => "Rogue",
        };
        let mut tags = Vec::new();
        if state.in_combat {
            tags.push("⚔".to_string());
        }
        if p.in_duel.unwrap_or(false) {
            tags.push("DUEL".into());
        }
        if let Some(title) = &p.title {
            tags.push(format!("\"{title}\""));
        }
        if let Some(guild) = &p.guild_name {
            tags.push(format!("<{guild}>"));
        }
        format!(
            "{}  Lv.{} {}  {}g  {}\nHP {} {}/{}  MP {} {}/{}  XP {} {}/{}",
            p.username,
            p.level,
            cls,
            p.gold,
            tags.join(" "),
            meter(p.hp, p.max_hp, 12),
            p.hp,
            p.max_hp,
            meter(p.mp, p.max_mp, 8),
            p.mp,
            p.max_mp,
            meter(p.xp, p.xp_to_level, 8),
            p.xp,
            p.xp_to_level
        )
    } else {
        "REALM OF ECHOES — awaiting hero...".into()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Realm of Echoes ");
    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, area);
}

fn render_log(f: &mut Frame, area: Rect, state: &TuiState) {
    let items: Vec<ListItem> = state
        .log_lines
        .iter()
        .map(|(text, style)| ListItem::new(Line::from(Span::styled(text.clone(), style_color(*style)))))
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" World "),
    );
    f.render_widget(list, area);
}

fn render_sidebar(f: &mut Frame, area: Rect, state: &TuiState) {
    let mut lines: Vec<Line> = Vec::new();

    if !state.client.authenticated && state.client.auth_step == AuthStep::Class {
        for line in class_select_text().lines().map(str::to_string) {
            lines.push(Line::from(line));
        }
    } else if !state.client.authenticated {
        lines.push(Line::from(Span::styled(
            "REALM OF ECHOES",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from("login    — return"));
        lines.push(Line::from("register — new hero"));
        lines.push(Line::from(""));
        lines.push(Line::from("n s e w  move"));
        lines.push(Line::from("l look  i inv"));
    } else if let (
        Some(target),
        Some(hp),
        Some(max_hp),
    ) = (
        state.combat_target.as_ref(),
        state.combat_target_hp,
        state.combat_target_max_hp,
    ) {
        lines.push(Line::from(Span::styled("Combat", Style::default().fg(Color::Red))));
        lines.push(Line::from(combat_target_line(target, hp, max_hp)));
        lines.push(Line::from(""));
    }

    if let Some(room) = &state.room {
        if let Some(art) = &room.zone_art {
            lines.push(Line::from(art.clone()));
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            room.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for line in wrap_text(&room.description, 28) {
            lines.push(Line::from(line));
        }
        if let Some(minimap) = &room.minimap {
            let map_lines: Vec<String> = render_minimap_ascii(minimap)
                .lines()
                .map(|line| line.to_string())
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Map", Style::default().fg(Color::Cyan))));
            for map_line in map_lines {
                lines.push(Line::from(map_line));
            }
        }
        if !room.exits.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Exits: {}", room.exits),
                Style::default().fg(Color::Yellow),
            )));
        }
        if !room.entities.is_empty() {
            lines.push(Line::from(Span::styled("Here", Style::default().fg(Color::Cyan))));
            for entity in &room.entities {
                let color = if entity.contains("[hostile]") {
                    Color::Red
                } else if entity.contains("(Lv.") {
                    Color::Yellow
                } else {
                    Color::Green
                };
                lines.push(Line::from(Span::styled(
                    format!("▸ {entity}"),
                    Style::default().fg(color),
                )));
            }
        }
    }

    if !state.online.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Online", Style::default().fg(Color::Cyan))));
        for p in state.online.iter().take(8) {
            lines.push(Line::from(format!(
                "{} L{} {}",
                p.username, p.level, p.zone
            )));
        }
    }

    if !state.ticker.is_empty() {
        lines.push(Line::from(format!("› {}", state.ticker)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("n s e w  move"));
    lines.push(Line::from("l look  i inv"));

    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(" Location "),
        );
    f.render_widget(paragraph, area);
}

fn render_input(f: &mut Frame, area: Rect, state: &TuiState) {
    let display = if state.client.hidden_input {
        "*".repeat(state.input.len())
    } else {
        state.input.clone()
    };
    let label = state.client.prompt.trim();
    let paragraph = Paragraph::new(display).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(format!(" {label} ")),
    );
    f.render_widget(paragraph, area);
}

fn style_color(style: OutputStyle) -> Style {
    let color = match style {
        OutputStyle::System => Color::Cyan,
        OutputStyle::Combat | OutputStyle::Death => Color::Red,
        OutputStyle::Chat => Color::Yellow,
        OutputStyle::Quest | OutputStyle::Global | OutputStyle::Epic => Color::Magenta,
        OutputStyle::Loot => Color::Green,
        OutputStyle::Party => Color::Blue,
        OutputStyle::Trade => Color::Rgb(255, 165, 0),
        OutputStyle::Emote => Color::Cyan,
        OutputStyle::Normal => Color::White,
    };
    let modifier = if matches!(style, OutputStyle::Death | OutputStyle::Epic) {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    Style::default().fg(color).add_modifier(modifier)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current = word.to_string();
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

