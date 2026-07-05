use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use axum::Router;
use serde::Serialize;
use futures_util::{SinkExt, StreamExt};
use realm_core::db::{
    create_player, find_player, hash_password, init_database, save_player, verify_password,
};
use realm_core::guilds::{find_guild_by_member, init_guilds};
use realm_core::types::class_stats;
use realm_core::{
    CommandCallbacks, CommandHandler, DuelManager, PartyManager, PlayerSession, TradeManager,
    World, WorldEventManager,
};
use realm_protocol::{ClassName, ClientMessage, OnlinePlayer, OutputStyle, ServerMessage};
use tokio::sync::{mpsc, Mutex};

#[derive(Clone)]
struct AppState {
    inner: Arc<GameServer>,
}

pub struct GameServer {
    world: World,
    players: Mutex<HashMap<String, PlayerSession>>,
    conn_tx: Mutex<HashMap<u64, mpsc::UnboundedSender<ServerMessage>>>,
    conn_user: Mutex<HashMap<u64, String>>,
    user_conn: Mutex<HashMap<String, u64>>,
    party: Mutex<PartyManager>,
    trade: Mutex<TradeManager>,
    duel: Mutex<DuelManager>,
    events: Mutex<WorldEventManager>,
    motd: String,
    next_conn: Mutex<u64>,
}

enum Delivery {
    ToPlayer {
        key: String,
        msg: ServerMessage,
        with_prompt: bool,
    },
    ToRoom {
        room: String,
        msg: ServerMessage,
        exclude: Option<String>,
    },
    RoomNotify {
        room: String,
        text: String,
        exclude: Option<String>,
    },
    BroadcastOnline,
    Global {
        text: String,
        style: OutputStyle,
    },
    Guild {
        guild_id: String,
        text: String,
    },
    Ticker {
        text: String,
    },
    Flash {
        key: String,
        color: String,
    },
}

impl GameServer {
    fn new(world_path: &Path, motd: String) -> Result<Self> {
        let world = World::load(world_path).context("load world.json")?;

        Ok(Self {
            world,
            players: Mutex::new(HashMap::new()),
            conn_tx: Mutex::new(HashMap::new()),
            conn_user: Mutex::new(HashMap::new()),
            user_conn: Mutex::new(HashMap::new()),
            party: Mutex::new(PartyManager::new()),
            trade: Mutex::new(TradeManager::new()),
            duel: Mutex::new(DuelManager::new()),
            events: Mutex::new(WorldEventManager::new(
                World::load(world_path).context("load world for events")?,
                |_| {},
            )),
            motd,
            next_conn: Mutex::new(1),
        })
    }

    async fn next_conn_id(&self) -> u64 {
        let mut n = self.next_conn.lock().await;
        let id = *n;
        *n += 1;
        id
    }

    async fn send_raw(&self, conn_id: u64, msg: ServerMessage) {
        let txs = self.conn_tx.lock().await;
        if let Some(tx) = txs.get(&conn_id) {
            let _ = tx.send(msg);
        }
    }

    async fn send_to_key(&self, key: &str, msg: ServerMessage, with_prompt: bool) {
        let user_conn = self.user_conn.lock().await;
        let Some(conn_id) = user_conn.get(key) else {
            return;
        };
        let needs_prompt =
            with_prompt && matches!(msg, ServerMessage::Output { .. } | ServerMessage::Error { .. });
        self.send_raw(*conn_id, msg).await;
        if needs_prompt {
            self.send_raw(*conn_id, ServerMessage::Prompt { text: ">".into() })
                .await;
        }
    }

    async fn flush_deliveries(&self, queue: Vec<Delivery>) {
        for item in queue {
            match item {
                Delivery::ToPlayer {
                    key,
                    msg,
                    with_prompt,
                } => {
                    self.send_to_key(&key, msg, with_prompt).await;
                }
                Delivery::ToRoom { room, msg, exclude } => {
                    let players = self.players.lock().await;
                    let user_conn = self.user_conn.lock().await;
                    for (key, session) in players.iter() {
                        if session.room_id() != room {
                            continue;
                        }
                        if exclude
                            .as_ref()
                            .is_some_and(|e| e.eq_ignore_ascii_case(session.username()))
                        {
                            continue;
                        }
                        if let Some(conn_id) = user_conn.get(key) {
                            let needs_prompt = matches!(
                                msg,
                                ServerMessage::Output { .. } | ServerMessage::Error { .. }
                            );
                            self.send_raw(*conn_id, msg.clone()).await;
                            if needs_prompt {
                                self.send_raw(
                                    *conn_id,
                                    ServerMessage::Prompt { text: ">".into() },
                                )
                                .await;
                            }
                        }
                    }
                }
                Delivery::RoomNotify { room, text, exclude } => {
                    let players = self.players.lock().await;
                    let user_conn = self.user_conn.lock().await;
                    for (key, session) in players.iter() {
                        if session.room_id() != room {
                            continue;
                        }
                        if exclude
                            .as_ref()
                            .is_some_and(|e| e.eq_ignore_ascii_case(session.username()))
                        {
                            continue;
                        }
                        if let Some(conn_id) = user_conn.get(key) {
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Output {
                                    text: text.clone(),
                                    style: Some(OutputStyle::Chat),
                                },
                            )
                            .await;
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Prompt { text: ">".into() },
                            )
                            .await;
                        }
                    }
                }
                Delivery::BroadcastOnline => {
                    self.broadcast_online().await;
                }
                Delivery::Global { text, style } => {
                    let players = self.players.lock().await;
                    let user_conn = self.user_conn.lock().await;
                    for (key, session) in players.iter() {
                        if !session.authenticated {
                            continue;
                        }
                        if let Some(conn_id) = user_conn.get(key) {
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Output {
                                    text: text.clone(),
                                    style: Some(style),
                                },
                            )
                            .await;
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Prompt { text: ">".into() },
                            )
                            .await;
                        }
                    }
                }
                Delivery::Guild { guild_id, text } => {
                    let players = self.players.lock().await;
                    let user_conn = self.user_conn.lock().await;
                    for (key, session) in players.iter() {
                        if !session.authenticated {
                            continue;
                        }
                        let Some(guild) = find_guild_by_member(session.username()) else {
                            continue;
                        };
                        if guild.id != guild_id {
                            continue;
                        }
                        if let Some(conn_id) = user_conn.get(key) {
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Output {
                                    text: text.clone(),
                                    style: Some(OutputStyle::Party),
                                },
                            )
                            .await;
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Prompt { text: ">".into() },
                            )
                            .await;
                        }
                    }
                }
                Delivery::Ticker { text } => {
                    let players = self.players.lock().await;
                    let user_conn = self.user_conn.lock().await;
                    for (key, session) in players.iter() {
                        if !session.authenticated {
                            continue;
                        }
                        if let Some(conn_id) = user_conn.get(key) {
                            self.send_raw(
                                *conn_id,
                                ServerMessage::Ticker {
                                    text: text.clone(),
                                },
                            )
                            .await;
                        }
                    }
                }
                Delivery::Flash { key, color } => {
                    self.send_to_key(
                        &key,
                        ServerMessage::Flash { color },
                        false,
                    )
                    .await;
                }
            }
        }
    }

    async fn broadcast_online(&self) {
        let list: Vec<OnlinePlayer> = {
            let players = self.players.lock().await;
            players
                .values()
                .filter(|p| p.authenticated)
                .map(|p| OnlinePlayer {
                    username: p.username().to_string(),
                    level: p.data.level,
                    class_name: p.data.class_name,
                    zone: self.world.get_zone(p.room_id()),
                })
                .collect()
        };

        let players = self.players.lock().await;
        let user_conn = self.user_conn.lock().await;
        for (key, session) in players.iter() {
            if !session.authenticated {
                continue;
            }
            if let Some(conn_id) = user_conn.get(key) {
                self.send_raw(
                    *conn_id,
                    ServerMessage::Online {
                        players: list.clone(),
                    },
                )
                .await;
            }
        }
    }

    async fn process_command(&self, conn_id: u64, input: &str) -> Option<Vec<Delivery>> {
        let key = {
            let conn_user = self.conn_user.lock().await;
            conn_user.get(&conn_id).cloned()?
        };

        let mut players = self.players.lock().await;
        if !players.get(&key).map(|p| p.authenticated).unwrap_or(false) {
            return None;
        }

        let mut party = self.party.lock().await;
        let mut trade = self.trade.lock().await;
        let mut duel = self.duel.lock().await;

        let mut commands = CommandHandler {
            world: self.world.clone(),
            party: std::mem::take(&mut *party),
            trade: std::mem::take(&mut *trade),
            duel: std::mem::take(&mut *duel),
        };

        let deliveries = {
            let queue: RefCell<Vec<Delivery>> = RefCell::new(Vec::new());

            let mut send = |u: &str, msg: ServerMessage| {
                queue.borrow_mut().push(Delivery::ToPlayer {
                    key: u.to_lowercase(),
                    msg,
                    with_prompt: true,
                });
            };

            let mut broadcast = |room: &str, msg: ServerMessage, ex: Option<&str>| {
                queue.borrow_mut().push(Delivery::ToRoom {
                    room: room.to_string(),
                    msg,
                    exclude: ex.map(|s| s.to_string()),
                });
            };

            let mut room_notify = |room: &str, text: &str, ex: Option<&str>| {
                queue.borrow_mut().push(Delivery::RoomNotify {
                    room: room.to_string(),
                    text: text.to_string(),
                    exclude: ex.map(|s| s.to_string()),
                });
            };

            let mut broadcast_online = || {
                queue.borrow_mut().push(Delivery::BroadcastOnline);
            };

            let mut flash = |u: &str, color: &str| {
                queue.borrow_mut().push(Delivery::Flash {
                    key: u.to_lowercase(),
                    color: color.to_string(),
                });
            };

            let mut global_broadcast = |text: &str, style: OutputStyle| {
                queue.borrow_mut().push(Delivery::Global {
                    text: text.to_string(),
                    style,
                });
            };

            let mut ticker_cb = |text: &str| {
                queue.borrow_mut().push(Delivery::Ticker {
                    text: text.to_string(),
                });
            };

            let mut guild_chat = |gid: &str, text: &str| {
                queue.borrow_mut().push(Delivery::Guild {
                    guild_id: gid.to_string(),
                    text: text.to_string(),
                });
            };

            let mut cb = CommandCallbacks {
                send: &mut send,
                broadcast: &mut broadcast,
                room_notify: &mut room_notify,
                broadcast_online: &mut broadcast_online,
                flash: &mut flash,
                global_broadcast: &mut global_broadcast,
                ticker: &mut ticker_cb,
                guild_chat: &mut guild_chat,
            };

            commands.handle(&key, &mut players, input, &mut cb);
            queue.into_inner()
        };

        *party = commands.party;
        *trade = commands.trade;
        *duel = commands.duel;

        if let Some(p) = players.get(&key) {
            let _ = save_player(&p.data);
        }

        Some(deliveries)
    }

    async fn post_login(&self, key: &str, conn_id: u64, welcome: String) {
        self.send_to_key(
            key,
            ServerMessage::Output {
                text: welcome,
                style: Some(OutputStyle::System),
            },
            true,
        )
        .await;
        self.send_to_key(
            key,
            ServerMessage::Motd {
                text: self.motd.clone(),
            },
            false,
        )
        .await;

        let (room, username) = {
            let players = self.players.lock().await;
            let Some(p) = players.get(key) else {
                return;
            };
            (p.room_id().to_string(), p.username().to_string())
        };
        self.flush_deliveries(vec![Delivery::RoomNotify {
            room,
            text: format!("{username} enters the realm."),
            exclude: None,
        }])
        .await;

        if let Some(deliveries) = self.process_command(conn_id, "look").await {
            self.flush_deliveries(deliveries).await;
        }

        let snapshot = {
            let players = self.players.lock().await;
            players.get(key).map(|p| p.to_snapshot(&self.world))
        };
        if let Some(player) = snapshot {
            self.send_to_key(key, ServerMessage::Stats { player }, false)
                .await;
        }

        let show_newbie_tip = {
            let players = self.players.lock().await;
            players
                .get(key)
                .map(|p| p.data.level == 1 && p.data.quests.is_empty())
                .unwrap_or(false)
        };
        if show_newbie_tip {
            self.send_to_key(
                key,
                ServerMessage::Output {
                    text: "New adventurer? Captain Aldric stands in the square — type \"talk aldric\" for quests, then \"accept goblin_menace\". Head south into the Whispering Woods and \"attack goblin\".".into(),
                    style: Some(OutputStyle::Quest),
                },
                false,
            )
            .await;
        }

        self.broadcast_online().await;
        self.send_to_key(key, ServerMessage::Prompt { text: ">".into() }, false)
            .await;
    }

    async fn handle_login(
        &self,
        conn_id: u64,
        username: String,
        password: String,
    ) -> Result<(), &'static str> {
        let stored = find_player(&username);
        let Some(stored) = stored else {
            return Err("Invalid username or password.");
        };
        if !verify_password(&password, &stored.password_hash) {
            return Err("Invalid username or password.");
        }

        let key = stored.username.to_lowercase();
        {
            let players = self.players.lock().await;
            if players.contains_key(&key) {
                return Err("That character is already logged in.");
            }
        }

        let mut session = PlayerSession::new(stored);
        session.authenticated = true;
        self.players.lock().await.insert(key.clone(), session);
        self.conn_user.lock().await.insert(conn_id, key.clone());
        self.user_conn.lock().await.insert(key.clone(), conn_id);

        self.post_login(&key, conn_id, format!("Welcome back, {username}!"))
            .await;
        Ok(())
    }

    async fn handle_register(
        &self,
        conn_id: u64,
        username: String,
        password: String,
        class_name: ClassName,
    ) -> Result<(), &'static str> {
        if username.len() < 3 || username.len() > 16 {
            return Err("Username must be 3-16 characters.");
        }
        if !username
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
            || !username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err("Username must start with a letter and contain only letters, numbers, underscores.");
        }
        if password.len() < 4 {
            return Err("Password must be at least 4 characters.");
        }
        if find_player(&username).is_some() {
            return Err("Username already taken.");
        }

        let cls = class_stats(class_name);
        let stored = create_player(
            username.clone(),
            hash_password(&password),
            class_name,
            (cls.max_hp, cls.max_mp),
        );
        let key = stored.username.to_lowercase();
        let mut session = PlayerSession::new(stored);
        session.authenticated = true;
        self.players.lock().await.insert(key.clone(), session);
        self.conn_user.lock().await.insert(conn_id, key.clone());
        self.user_conn.lock().await.insert(key.clone(), conn_id);

        self.send_to_key(
            &key,
            ServerMessage::Output {
                text: format!(
                    "Character created! You are {}.\n{}",
                    cls.display_name, cls.description
                ),
                style: Some(OutputStyle::System),
            },
            true,
        )
        .await;

        self.post_login(
            &key,
            conn_id,
            "Type \"help\" for commands. Good luck, adventurer!".into(),
        )
        .await;
        Ok(())
    }

    async fn disconnect(&self, conn_id: u64) {
        let key = self.conn_user.lock().await.remove(&conn_id);
        self.conn_tx.lock().await.remove(&conn_id);

        if let Some(key) = key {
            self.user_conn.lock().await.remove(&key);

            let fled_msg = {
                let mut players = self.players.lock().await;
                let fled = players.get(&key).map(|s| {
                    let fled_name = s.username().to_string();
                    let target_key = s.pvp_target.clone();
                    (fled_name, target_key)
                });

                if let Some((fled_name, Some(target_key))) = fled {
                    if let Some(opponent) = players.get_mut(&target_key) {
                        opponent.clear_combat();
                        Some((target_key, fled_name))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some((opponent_key, fled_name)) = fled_msg {
                self.send_to_key(
                    &opponent_key,
                    ServerMessage::Output {
                        text: format!("{fled_name} fled the battle!"),
                        style: Some(OutputStyle::Combat),
                    },
                    true,
                )
                .await;
            }

            let (room, username) = {
                let mut players = self.players.lock().await;
                let Some(session) = players.remove(&key) else {
                    return;
                };
                self.party.lock().await.on_disconnect(&session);
                self.trade.lock().await.on_disconnect(session.username());
                self.duel.lock().await.on_disconnect(session.username());
                let _ = save_player(&session.data);
                (session.room_id().to_string(), session.username().to_string())
            };

            self.flush_deliveries(vec![Delivery::RoomNotify {
                room,
                text: format!("{username} has left the realm."),
                exclude: None,
            }])
            .await;
            self.broadcast_online().await;
        }
    }
}

pub async fn run(port: u16) -> Result<()> {
    let data_dir = std::env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"));

    let world_path = std::env::var("WORLD_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/world.json"));

    let motd = std::env::var("MOTD").unwrap_or_else(|_| {
        std::fs::read_to_string(data_dir.join("motd.txt"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "Welcome to the Realm of Echoes! Type help for commands.".into())
    });

    init_database().await.context("init database")?;
    init_guilds().context("init guilds")?;

    let server = Arc::new(GameServer::new(&world_path, motd)?);

    {
        let server_events = server.clone();
        let mut events = server.events.lock().await;
        *events = WorldEventManager::new(server.world.clone(), move |text| {
            let s = server_events.clone();
            tokio::spawn(async move {
                s.flush_deliveries(vec![Delivery::Global {
                    text,
                    style: OutputStyle::Global,
                }])
                .await;
            });
        });
        events.start();
    }

    let save_server = server.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let players = save_server.players.lock().await;
            for p in players.values() {
                let _ = save_player(&p.data);
            }
        }
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/config", get(config_handler))
        .route("/health", get(|| async { "ok" }))
        .with_state(AppState {
            inner: server.clone(),
        });

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Realm of Echoes server listening on ws://0.0.0.0:{port}/ws");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Serialize)]
struct ServerConfig {
    #[serde(rename = "wsUrl")]
    ws_url: String,
    #[serde(rename = "apiUrl")]
    api_url: String,
}

async fn config_handler() -> Json<ServerConfig> {
    let domain = std::env::var("RAILWAY_PUBLIC_DOMAIN")
        .or_else(|_| std::env::var("PUBLIC_DOMAIN"))
        .unwrap_or_else(|_| "localhost:4242".into());

    let secure = !domain.starts_with("localhost") && !domain.starts_with("127.0.0.1");
    let ws_scheme = if secure { "wss" } else { "ws" };
    let http_scheme = if secure { "https" } else { "http" };

    Json(ServerConfig {
        ws_url: format!("{ws_scheme}://{domain}/ws"),
        api_url: format!("{http_scheme}://{domain}"),
    })
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(app): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app.inner))
}

async fn handle_socket(socket: WebSocket, server: Arc<GameServer>) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();

    let conn_id = server.next_conn_id().await;
    server.conn_tx.lock().await.insert(conn_id, tx.clone());

    let _ = tx.send(ServerMessage::Banner);
    let _ = tx.send(ServerMessage::Prompt {
        text: "login or register?".into(),
    });

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                let client_msg: ClientMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => {
                        server
                            .send_raw(
                                conn_id,
                                ServerMessage::Error {
                                    text: "Invalid message format.".into(),
                                },
                            )
                            .await;
                        continue;
                    }
                };

                let authenticated = server
                    .conn_user
                    .lock()
                    .await
                    .contains_key(&conn_id);

                match client_msg {
                    ClientMessage::Login { username, password } if !authenticated => {
                        if let Err(err) = server.handle_login(conn_id, username, password).await {
                            server
                                .send_raw(conn_id, ServerMessage::Error { text: err.into() })
                                .await;
                            server
                                .send_raw(
                                    conn_id,
                                    ServerMessage::Prompt {
                                        text: "login or register?".into(),
                                    },
                                )
                                .await;
                        }
                    }
                    ClientMessage::Register {
                        username,
                        password,
                        class_name,
                    } if !authenticated => {
                        if let Err(err) = server
                            .handle_register(conn_id, username, password, class_name)
                            .await
                        {
                            server
                                .send_raw(conn_id, ServerMessage::Error { text: err.into() })
                                .await;
                            server
                                .send_raw(
                                    conn_id,
                                    ServerMessage::Prompt {
                                        text: "login or register?".into(),
                                    },
                                )
                                .await;
                        }
                    }
                    ClientMessage::Command { input } if authenticated => {
                        if let Some(deliveries) = server.process_command(conn_id, &input).await {
                            server.flush_deliveries(deliveries).await;
                        }
                    }
                    _ => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
    server.disconnect(conn_id).await;
}