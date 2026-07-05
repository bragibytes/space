use std::collections::HashMap;

use realm_protocol::{CombatSnapshot, OutputStyle, ServerMessage};

use crate::achievement_service::{check_achievements, format_achievements, AchievementTriggers};
use crate::combat::{
    handle_player_death, is_safe_zone, player_ability, player_ability_vs_player, player_attack,
    player_attack_player, pvp_victory_xp,
};
use crate::db::get_all_players;
use crate::duel::DuelManager;
use crate::items::ITEMS;
use crate::minimap::build_minimap;
use crate::party::PartyManager;
use crate::player::PlayerSession;
use crate::quests::{check_quest_complete, create_quest_progress, format_quest_progress, QuestStatus};
use crate::social::{
    find_item_in_inventory, find_player_key, handle_admin_command, handle_craft_command,
    handle_duel_command, handle_guild_command, handle_party_command, handle_trade_command,

};
use crate::trade::TradeManager;
use crate::types::{
    class_stats, Direction, DIRECTION_ALIASES, LOCKED_EXITS, ZONE_ART,
};
use crate::world::World;

pub struct CommandCallbacks<'a> {
    pub send: &'a mut dyn FnMut(&str, ServerMessage),
    pub broadcast: &'a mut dyn FnMut(&str, ServerMessage, Option<&str>),
    pub room_notify: &'a mut dyn FnMut(&str, &str, Option<&str>),
    pub broadcast_online: &'a mut dyn FnMut(),
    pub flash: &'a mut dyn FnMut(&str, &str),
    pub global_broadcast: &'a mut dyn FnMut(&str, OutputStyle),
    pub ticker: &'a mut dyn FnMut(&str),
    pub guild_chat: &'a mut dyn FnMut(&str, &str),
}

pub struct CommandHandler {
    pub world: World,
    pub party: PartyManager,
    pub trade: TradeManager,
    pub duel: DuelManager,
}

impl CommandHandler {
    pub fn handle(
        &mut self,
        player_key: &str,
        players: &mut HashMap<String, PlayerSession>,
        input: &str,
        cb: &mut CommandCallbacks,
    ) {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();
        let args: Vec<&str> = parts[1..].to_vec();
        let arg_str = args.join(" ");

        match cmd.as_str() {
            "look" | "l" => self.look(player_key, players, cb),
            "north" | "n" => self.move_dir(player_key, players, Direction::North, cb),
            "south" | "s" => self.move_dir(player_key, players, Direction::South, cb),
            "east" | "e" => self.move_dir(player_key, players, Direction::East, cb),
            "west" | "w" => self.move_dir(player_key, players, Direction::West, cb),
            "up" | "u" => self.move_dir(player_key, players, Direction::Up, cb),
            "down" | "d" => self.move_dir(player_key, players, Direction::Down, cb),
            "say" => self.say(player_key, players, &arg_str, cb),
            "yell" => self.yell(player_key, players, &arg_str, cb),
            "whisper" | "tell" => {
                let target = args.first().copied().unwrap_or("");
                let msg = args.get(1..).map(|s| s.join(" ")).unwrap_or_default();
                self.whisper(player_key, players, target, &msg, cb);
            }
            "attack" | "kill" => self.attack(player_key, players, &arg_str, cb),
            "ability" | "cast" => self.use_ability(player_key, players, 1, cb),
            "special" => self.use_ability(player_key, players, 2, cb),
            "party" => {
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                let mut notify = |r: &str, t: &str, e: Option<&str>| (cb.room_notify)(r, t, e);
                handle_party_command(player_key, &args, &mut self.party, players, &mut send, &mut notify);
            }
            "p" => {
                let party_args: Vec<&str> = if args.is_empty() {
                    vec![]
                } else {
                    let mut v = vec!["say"];
                    v.extend(args.iter().copied());
                    v
                };
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                let mut notify = |r: &str, t: &str, e: Option<&str>| (cb.room_notify)(r, t, e);
                handle_party_command(player_key, &party_args, &mut self.party, players, &mut send, &mut notify);
            }
            "trade" => {
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                handle_trade_command(player_key, &args, &mut self.trade, players, &mut send);
            }
            "duel" => {
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                let mut notify = |r: &str, t: &str, e: Option<&str>| (cb.room_notify)(r, t, e);
                handle_duel_command(player_key, &args, &mut self.duel, players, &mut send, &mut notify);
            }
            "craft" => {
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                handle_craft_command(player_key, &arg_str, &self.world, players, &mut send);
            }
            "admin" => {
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                let mut online = || (cb.broadcast_online)();
                handle_admin_command(player_key, &args, &self.world, players, &mut send, &mut online);
            }
            "guild" => {
                let mut send = |k: &str, m: ServerMessage| (cb.send)(k, m);
                let mut gchat = |gid: &str, text: &str, ex: Option<&str>| {
                    let _ = ex;
                    (cb.guild_chat)(gid, text);
                };
                handle_guild_command(player_key, &args, players, &mut send, &mut gchat);
            }
            "get" | "take" => self.get_item(player_key, players, &arg_str, cb),
            "drop" => self.drop_item(player_key, players, &arg_str, cb),
            "inventory" | "inv" | "i" => self.show_inventory(player_key, players, cb),
            "equip" => self.equip(player_key, players, &arg_str, cb),
            "use" => self.use_item(player_key, players, &arg_str, cb),
            "stats" | "score" => self.show_stats(player_key, players, cb),
            "who" => (cb.broadcast_online)(),
            "quest" | "quests" => self.show_quests(player_key, players, cb),
            "talk" | "greet" => self.talk(player_key, players, &arg_str, cb),
            "accept" => self.accept_quest(player_key, players, &arg_str, cb),
            "complete" | "turnin" => self.complete_quest(player_key, players, &arg_str, cb),
            "buy" => self.buy(player_key, players, &arg_str, cb),
            "rest" => self.rest(player_key, players, cb),
            "help" | "h" => self.help(player_key, players, cb),
            "quit" | "exit" => (cb.send)(player_key, ServerMessage::Disconnect { reason: "Farewell, adventurer!".into() }),
            "global" | "g" => self.global(player_key, players, &arg_str, cb),
            "emote" | "me" => self.emote(player_key, players, &arg_str, cb),
            "search" => self.search(player_key, players, cb),
            "achievements" | "ach" => {
                if let Some(p) = players.get(player_key) {
                    out(cb, player_key, format_achievements(p), OutputStyle::Quest);
                }
            }
            "leaderboard" | "lb" => self.leaderboard(player_key, cb),
            other => {
                if let Some(dir) = DIRECTION_ALIASES.get(other) {
                    self.move_dir(player_key, players, *dir, cb);
                } else {
                    out(cb, player_key, format!("Unknown command: \"{cmd}\". Type 'help' for commands."), OutputStyle::System);
                }
            }
        }
        self.finalize(player_key, players, cb);
    }

    fn finalize(&self, player_key: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let (room, gold, level) = {
            let Some(player) = players.get(player_key) else { return };
            (player.room_id().to_string(), player.data.gold, player.data.level)
        };
        let (party_leader, in_duel) = {
            let Some(p) = players.get(player_key) else { return };
            let leader = self.party.get_leader_key(p).and_then(|k| players.get(&k).map(|x| x.username().to_string()));
            (leader, self.duel.is_in_duel(p.username()))
        };
        if let Some(player) = players.get_mut(player_key) {
            player.party_leader = party_leader;
            player.in_duel = in_duel;
        }
        if let Some(player) = players.get_mut(player_key) {
            let triggers = AchievementTriggers {
                room: Some(room),
                gold: Some(gold),
                level: Some(level),
                ..Default::default()
            };
            check_achievements(player, |key, msg| (cb.send)(key, msg), triggers);
        }
    }

    fn look(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let Some(player) = players.get(pk) else { return };
        let Some(room) = self.world.get_room(player.room_id()) else { return };
        let mobs = self.world.mobs();
        let npcs = self.world.npcs();

        let exits: Vec<String> = room.template.exits.keys().map(|k| k.to_string()).collect();
        let mut entities = Vec::new();

        for mob in &room.mobs {
            if let Some(tmpl) = mobs.get(&mob.template_id) {
                let mut tags = Vec::new();
                if mob.elite.unwrap_or(false) || tmpl.elite { tags.push("ELITE"); }
                if tmpl.boss { tags.push("BOSS"); }
                let tag_str = if tags.is_empty() { String::new() } else { format!(" [{}]", tags.join(" ")) };
                entities.push(format!("{} (Lv.{}){} [hostile]", tmpl.name, tmpl.level, tag_str));
            }
        }
        for npc_id in &room.npcs {
            if let Some(npc) = npcs.get(npc_id) {
                entities.push(npc.name.clone());
            }
        }
        for (key, p) in players.iter() {
            if key != pk && p.room_id() == player.room_id() {
                let title = p.data.title.as_ref().map(|t| format!(" \"{t}\"")).unwrap_or_default();
                let cls = class_stats(p.data.class_name);
                entities.push(format!("{}{} (Lv.{} {})", p.username(), title, p.data.level, cls.display_name));
            }
        }
        for item_id in &room.items {
            entities.push(ITEMS.get(item_id).map(|i| i.name.clone()).unwrap_or_else(|| item_id.clone()));
        }

        let zone = room.template.zone.clone();
        (cb.send)(pk, ServerMessage::Room {
            title: room.template.name.clone(),
            description: room.template.description.clone(),
            exits: exits.join(", "),
            entities,
            zone: Some(zone.clone()),
            minimap: Some(build_minimap(&self.world.rooms(), player.room_id())),
            zone_art: ZONE_ART.get(zone.as_str()).map(|s| s.to_string()),
        });
        if player.in_combat() {
            self.send_combat_snapshot(pk, players, cb);
        }
    }

    fn move_dir(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, dir: Direction, cb: &mut CommandCallbacks) {
        let dir_str = dir.as_str();
        let (room_id, username, in_combat) = {
            let Some(p) = players.get(pk) else { return };
            (p.room_id().to_string(), p.username().to_string(), p.in_combat())
        };
        if in_combat {
            out(cb, pk, "You cannot flee while in combat! Use \"attack\" or defeat your foe.", OutputStyle::Combat);
            return;
        }
        let Some(room) = self.world.get_room(&room_id) else { return };
        let dest_id = room.template.exits.get(dir_str).cloned();
        let Some(dest_id) = dest_id else {
            out(cb, pk, format!("You cannot go {dir_str}."), OutputStyle::System);
            return;
        };
        if let Some(lock) = LOCKED_EXITS.get(dest_id.as_str()) {
            let count = players.get(pk).map(|p| p.count_item(&lock.item)).unwrap_or(0);
            if count < 1 {
                out(cb, pk, lock.message.clone(), OutputStyle::System);
                return;
            }
        }
        if self.world.get_room(&dest_id).is_none() {
            return;
        }
        let old_zone = self.world.get_zone(&room_id);
        {
            let Some(p) = players.get_mut(pk) else { return };
            p.data.room_id = dest_id.clone();
        }
        (cb.room_notify)(&room_id, &format!("{username} heads {dir_str}."), Some(&username));
        (cb.room_notify)(&dest_id, &format!("{username} arrives from the {}.", opposite_dir(dir_str)), Some(&username));
        if self.world.get_zone(&dest_id) != old_zone {
            (cb.broadcast_online)();
        }
        // visit quests
        let quests = self.world.quests();
        if let Some(player) = players.get_mut(pk) {
            for qp in &mut player.data.quests {
                if qp.status != QuestStatus::Active {
                    continue;
                }
                let Some(quest) = quests.get(&qp.quest_id) else { continue };
                for obj in &quest.objectives {
                    if obj.objective_type == "visit" && obj.target == dest_id {
                        qp.progress.insert(obj.target.clone(), 1);
                        if check_quest_complete(quest, qp) {
                            qp.status = QuestStatus::Complete;
                            out(cb, pk, format!("Quest complete: {}! Return to the quest giver.", quest.name), OutputStyle::Quest);
                        }
                    }
                }
            }
        }
        self.look(pk, players, cb);
    }

    fn say(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, message: &str, cb: &mut CommandCallbacks) {
        if message.is_empty() {
            out(cb, pk, "Say what? Usage: say <message>", OutputStyle::System);
            return;
        }
        let (room_id, username) = {
            let Some(p) = players.get(pk) else { return };
            (p.room_id().to_string(), p.username().to_string())
        };
        (cb.room_notify)(&room_id, &format!("{username} says: \"{message}\""), None);
        let _ = players;
    }

    fn yell(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, message: &str, cb: &mut CommandCallbacks) {
        if message.is_empty() {
            out(cb, pk, "Yell what? Usage: yell <message>", OutputStyle::System);
            return;
        }
        let (zone, username) = {
            let Some(p) = players.get(pk) else { return };
            (self.world.get_zone(p.room_id()), p.username().to_string())
        };
        for (key, p) in players.iter() {
            if self.world.get_zone(p.room_id()) == zone {
                out(cb, key, format!("[{zone}] {username} yells: \"{message}\""), OutputStyle::Chat);
            }
        }
    }

    fn whisper(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, target: &str, message: &str, cb: &mut CommandCallbacks) {
        if target.is_empty() || message.is_empty() {
            out(cb, pk, "Usage: whisper <player> <message>", OutputStyle::System);
            return;
        }
        let Some(target_key) = find_player_key(players, players.get(pk).map(|p| p.username()).unwrap_or(""), target) else {
            out(cb, pk, format!("{target} is not online."), OutputStyle::System);
            return;
        };
        let (username, target_name) = {
            let p = players.get(pk).unwrap();
            let t = players.get(&target_key).unwrap();
            (p.username().to_string(), t.username().to_string())
        };
        out(cb, pk, format!("You whisper to {target_name}: \"{message}\""), OutputStyle::Chat);
        out(cb, &target_key, format!("{username} whispers: \"{message}\""), OutputStyle::Chat);
    }

    fn attack(&mut self, pk: &str, players: &mut HashMap<String, PlayerSession>, target: &str, cb: &mut CommandCallbacks) {
        if target.is_empty() {
            if let Some(player) = players.get(pk) {
                if let Some(opp_key) = player.pvp_target.clone() {
                    if let Some(opp) = players.get(&opp_key) {
                        if opp.room_id() == player.room_id() {
                            self.apply_pvp_round(pk, &opp_key, players, cb);
                            return;
                        }
                    }
                    players.get_mut(pk).unwrap().pvp_target = None;
                }
            }
            out(cb, pk, "Attack what? Usage: attack <target>", OutputStyle::System);
            return;
        }
        if let Some(def_key) = self.find_player_in_room(pk, players, target) {
            self.attack_player(pk, &def_key, players, cb);
            return;
        }
        if players.get(pk).map(|p| p.pvp_target.is_some()).unwrap_or(false) {
            out(cb, pk, "You are in PvP combat! Finish your fight first.", OutputStyle::Combat);
            return;
        }
        self.attack_mob(pk, players, target, cb);
    }

    fn attack_mob(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, target: &str, cb: &mut CommandCallbacks) {
        let mob_info = {
            let player = players.get(pk).unwrap();
            let room = self.world.get_room(player.room_id()).unwrap();
            let mobs = self.world.mobs();
            let lower = target.to_lowercase();
            room.mobs.iter().find(|m| {
                let tmpl = mobs.get(&m.template_id).unwrap();
                tmpl.name.to_lowercase().contains(&lower) || m.template_id.contains(&lower)
            }).map(|m| (m.instance_id.clone(), m.template_id.clone(), player.room_id().to_string()))
        };
        let Some((instance_id, template_id, room_id)) = mob_info else {
            out(cb, pk, format!("No \"{target}\" here to attack."), OutputStyle::Combat);
            return;
        };
        let mob_name = self.world.mobs().get(&template_id).unwrap().name.clone();
        players.get_mut(pk).unwrap().combat_target = Some(instance_id.clone());
        out(cb, pk, format!("You engage {mob_name} in combat!"), OutputStyle::Combat);

        let result = {
            let player = players.get_mut(pk).unwrap();
            self.world.get_room_mut(&room_id, |room| {
                let mob = room.mobs.iter_mut().find(|m| m.instance_id == instance_id).unwrap();
                player_attack(player, mob, &self.world, &mut [])
            }).unwrap()
        };

        for msg in &result.messages {
            let style = if msg.contains("LEVEL UP") { OutputStyle::Quest } else { OutputStyle::Combat };
            out(cb, pk, msg.clone(), style);
            if msg.contains("LEVEL UP") {
                (cb.send)(pk, ServerMessage::Bell);
                (cb.flash)(pk, "green");
            }
        }
        (cb.flash)(pk, "red");
        for peer_key in self.party_peers_in_room(pk, players) {
            if let Some(peer) = players.get(&peer_key) {
                (cb.send)(&peer_key, ServerMessage::Stats { player: peer.to_snapshot(&self.world) });
            }
        }
        if result.player_died {
            let msgs = handle_player_death(players.get_mut(pk).unwrap(), None);
            for msg in msgs { out(cb, pk, msg, OutputStyle::Death); }
            self.look(pk, players, cb);
        } else if result.mob_killed {
            players.get_mut(pk).unwrap().combat_target = None;
            (cb.room_notify)(&room_id, &format!("{} slays {mob_name}!", players.get(pk).unwrap().username()), None);
            (cb.ticker)(&format!("{} slays {mob_name}", players.get(pk).unwrap().username()));
            if let Some(player) = players.get_mut(pk) {
                check_achievements(player, |k, m| (cb.send)(k, m), AchievementTriggers { kill: Some(template_id), ..Default::default() });
            }
        }
        if let Some(player) = players.get(pk) {
            (cb.send)(pk, ServerMessage::Stats { player: player.to_snapshot(&self.world) });
        }
        self.send_combat_snapshot(pk, players, cb);
    }

    fn attack_player(&mut self, pk: &str, def_key: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let (att_name, def_name, room_id) = {
            let a = players.get(pk).unwrap();
            let d = players.get(def_key).unwrap();
            (a.username().to_string(), d.username().to_string(), a.room_id().to_string())
        };
        let duel_opp = self.duel.get_opponent(players.get(pk).unwrap().username());
        let in_duel = players.get(pk).unwrap().in_duel || players.get(def_key).unwrap().in_duel;
        if in_duel {
            if duel_opp.as_deref() != Some(def_key) {
                out(cb, pk, "You can only attack your duel opponent.", OutputStyle::Combat);
                return;
            }
        } else if is_safe_zone(&self.world, &room_id) {
            out(cb, pk, "PvP is disabled in town. Venture into the wilds, or challenge someone to a duel.", OutputStyle::System);
            return;
        }
        let first = players.get(pk).unwrap().pvp_target.is_none();
        players.get_mut(pk).unwrap().pvp_target = Some(def_key.to_string());
        players.get_mut(def_key).unwrap().pvp_target = Some(pk.to_string());
        players.get_mut(pk).unwrap().combat_target = None;
        players.get_mut(def_key).unwrap().combat_target = None;
        if first {
            out(cb, pk, format!("You attack {def_name}!"), OutputStyle::Combat);
            out(cb, def_key, format!("{att_name} attacks you! Fight back with \"attack {att_name}\"!"), OutputStyle::Combat);
            (cb.room_notify)(&room_id, &format!("{att_name} attacks {def_name}!"), None);
        }
        self.apply_pvp_round(pk, def_key, players, cb);
        (cb.flash)(pk, "red");
        (cb.flash)(def_key, "red");
    }

    fn apply_pvp_round(&mut self, pk: &str, def_key: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let mut attacker = players.remove(pk).unwrap();
        let mut defender = players.remove(def_key).unwrap();
        let result = player_attack_player(&mut attacker, &mut defender);
        players.insert(pk.to_string(), attacker);
        players.insert(def_key.to_string(), defender);
        for msg in &result.attacker_messages { out(cb, pk, msg.clone(), OutputStyle::Combat); }
        for msg in &result.defender_messages { out(cb, def_key, msg.clone(), OutputStyle::Combat); }
        if let Some(a) = players.get(pk) {
            (cb.send)(pk, ServerMessage::Stats { player: a.to_snapshot(&self.world) });
        }
        if let Some(d) = players.get(def_key) {
            (cb.send)(def_key, ServerMessage::Stats { player: d.to_snapshot(&self.world) });
        }
        if result.defender_died {
            self.resolve_pvp_victory(pk, def_key, players, cb);
        } else if result.attacker_died {
            self.resolve_pvp_victory(def_key, pk, players, cb);
        } else {
            self.send_combat_snapshot(pk, players, cb);
            self.send_combat_snapshot(def_key, players, cb);
        }
    }

    fn resolve_pvp_victory(&mut self, winner_key: &str, loser_key: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let room_id = players.get(winner_key).unwrap().room_id().to_string();
        let (winner_name, loser_name, loser_level) = {
            let w = players.get(winner_key).unwrap();
            let l = players.get(loser_key).unwrap();
            (w.username().to_string(), l.username().to_string(), l.data.level)
        };
        players.get_mut(winner_key).unwrap().clear_combat();
        players.get_mut(loser_key).unwrap().clear_combat();
        players.get_mut(winner_key).unwrap().in_duel = false;
        players.get_mut(loser_key).unwrap().in_duel = false;
        self.duel.end_duel(&winner_name, &loser_name);
        let death_msgs = handle_player_death(players.get_mut(loser_key).unwrap(), Some(&winner_name));
        for msg in death_msgs { out(cb, loser_key, msg, OutputStyle::Death); }
        self.look(loser_key, players, cb);
        if let Some(l) = players.get(loser_key) {
            (cb.send)(loser_key, ServerMessage::Stats { player: l.to_snapshot(&self.world) });
        }
        out(cb, winner_key, format!("*** You have slain {loser_name}! ***"), OutputStyle::Combat);
        let xp_msgs = players.get_mut(winner_key).unwrap().add_xp(pvp_victory_xp(loser_level));
        for msg in xp_msgs { out(cb, winner_key, msg, OutputStyle::Combat); }
        (cb.room_notify)(&room_id, &format!("{winner_name} has slain {loser_name} in PvP!"), None);
        (cb.ticker)(&format!("{winner_name} slays {loser_name}"));
        if let Some(w) = players.get_mut(winner_key) {
            check_achievements(w, |k, m| (cb.send)(k, m), AchievementTriggers { duel_win: Some(true), ..Default::default() });
        }
        if let Some(w) = players.get(winner_key) {
            (cb.send)(winner_key, ServerMessage::Stats { player: w.to_snapshot(&self.world) });
        }
        self.send_combat_snapshot(winner_key, players, cb);
        self.send_combat_snapshot(loser_key, players, cb);
    }

    fn use_ability(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, slot: u8, cb: &mut CommandCallbacks) {
        if let Some(opp_key) = players.get(pk).and_then(|p| p.pvp_target.clone()) {
            if let Some(opp) = players.get(&opp_key) {
                if opp.room_id() == players.get(pk).unwrap().room_id() {
                    let mut attacker = players.remove(pk).unwrap();
                    let mut defender = players.remove(&opp_key).unwrap();
                    let result = player_ability_vs_player(&mut attacker, &mut defender, slot);
                    players.insert(pk.to_string(), attacker);
                    players.insert(opp_key.clone(), defender);
                    for msg in &result.attacker_messages { out(cb, pk, msg.clone(), OutputStyle::Combat); }
                    for msg in &result.defender_messages { out(cb, &opp_key, msg.clone(), OutputStyle::Combat); }
                    (cb.flash)(pk, "yellow");
                    (cb.flash)(&opp_key, "yellow");
                    self.send_combat_snapshot(pk, players, cb);
                    self.send_combat_snapshot(&opp_key, players, cb);
                    return;
                }
            }
            players.get_mut(pk).unwrap().pvp_target = None;
            out(cb, pk, "Your opponent is gone.", OutputStyle::Combat);
            return;
        }
        let combat_target = players.get(pk).and_then(|p| p.combat_target.clone());
        let Some(instance_id) = combat_target else {
            out(cb, pk, "You must be in combat to use your ability.", OutputStyle::Combat);
            return;
        };
        let room_id = players.get(pk).unwrap().room_id().to_string();
        let result = self.world.get_room_mut(&room_id, |room| {
            let mob = room.mobs.iter_mut().find(|m| m.instance_id == instance_id);
            let Some(mob) = mob else { return None };
            let player = players.get_mut(pk).unwrap();
            Some(player_ability(player, mob, &self.world, slot))
        });
        let Some(Some(result)) = result else {
            players.get_mut(pk).unwrap().combat_target = None;
            out(cb, pk, "Your target is gone.", OutputStyle::Combat);
            return;
        };
        for msg in &result.messages { out(cb, pk, msg.clone(), OutputStyle::Combat); }
        (cb.flash)(pk, "yellow");
        if result.player_died {
            let msgs = handle_player_death(players.get_mut(pk).unwrap(), None);
            for msg in msgs { out(cb, pk, msg, OutputStyle::Death); }
            self.look(pk, players, cb);
        }
        if let Some(p) = players.get(pk) {
            (cb.send)(pk, ServerMessage::Stats { player: p.to_snapshot(&self.world) });
        }
        self.send_combat_snapshot(pk, players, cb);
    }

    fn get_item(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, item_name: &str, cb: &mut CommandCallbacks) {
        if item_name.is_empty() {
            out(cb, pk, "Get what? Usage: get <item>", OutputStyle::System);
            return;
        }
        let room_id = players.get(pk).unwrap().room_id().to_string();
        let lower = item_name.to_lowercase();
        let item_id = self.world.get_room_mut(&room_id, |room| {
            room.items.iter().position(|id| {
                ITEMS.get(id).map(|i| i.name.to_lowercase().contains(&lower) || id.to_lowercase().contains(&lower)).unwrap_or(false)
            }).map(|idx| room.items.remove(idx))
        }).flatten();
        let Some(item_id) = item_id else {
            out(cb, pk, format!("No \"{item_name}\" here."), OutputStyle::System);
            return;
        };
        let display = ITEMS.get(&item_id).map(|i| i.name.clone()).unwrap_or(item_id.clone());
        players.get_mut(pk).unwrap().add_item(&item_id);
        out(cb, pk, format!("You pick up {display}."), OutputStyle::Loot);
    }

    fn drop_item(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, item_name: &str, cb: &mut CommandCallbacks) {
        if item_name.is_empty() {
            out(cb, pk, "Drop what? Usage: drop <item>", OutputStyle::System);
            return;
        }
        let item_id = find_item_in_inventory(players.get(pk).unwrap(), item_name);
        let Some(item_id) = item_id else {
            out(cb, pk, format!("You don't have \"{item_name}\"."), OutputStyle::System);
            return;
        };
        if !players.get_mut(pk).unwrap().remove_item(&item_id) {
            return;
        }
        let room_id = players.get(pk).unwrap().room_id().to_string();
        let username = players.get(pk).unwrap().username().to_string();
        self.world.get_room_mut(&room_id, |room| room.items.push(item_id.clone()));
        let display = ITEMS.get(&item_id).map(|i| i.name.clone()).unwrap_or(item_id);
        out(cb, pk, format!("You drop {display}."), OutputStyle::System);
        (cb.room_notify)(&room_id, &format!("{username} drops {display}."), Some(&username));
    }

    fn show_inventory(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let player = players.get(pk).unwrap();
        if player.data.inventory.is_empty() {
            out(cb, pk, "Your inventory is empty.", OutputStyle::System);
            return;
        }
        let mut counts: HashMap<&str, u32> = HashMap::new();
        for id in &player.data.inventory {
            *counts.entry(id.as_str()).or_insert(0) += 1;
        }
        let mut lines = vec!["-- Inventory --".to_string()];
        for (id, count) in counts {
            let item = ITEMS.get(id);
            let equipped = if player.data.equipment.weapon.as_deref() == Some(id) || player.data.equipment.armor.as_deref() == Some(id) {
                " [equipped]"
            } else { "" };
            let name = item.map(|i| i.name.as_str()).unwrap_or(id);
            lines.push(format!("  {name}{}{}", if count > 1 { format!(" x{count}") } else { String::new() }, equipped));
        }
        lines.push(format!("Gold: {}", player.data.gold));
        out(cb, pk, lines.join("\n"), OutputStyle::System);
    }

    fn equip(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, item_name: &str, cb: &mut CommandCallbacks) {
        if item_name.is_empty() {
            out(cb, pk, "Equip what? Usage: equip <item>", OutputStyle::System);
            return;
        }
        let item_id = find_item_in_inventory(players.get(pk).unwrap(), item_name);
        let Some(item_id) = item_id else {
            out(cb, pk, format!("You don't have \"{item_name}\"."), OutputStyle::System);
            return;
        };
        let item = ITEMS.get(&item_id).cloned();
        let Some(item) = item else { return };
        let Some(slot) = item.slot else {
            out(cb, pk, format!("{} cannot be equipped.", item.name), OutputStyle::System);
            return;
        };
        let player = players.get_mut(pk).unwrap();
        match slot.as_str() {
            "weapon" => player.data.equipment.weapon = Some(item_id),
            "armor" => player.data.equipment.armor = Some(item_id),
            _ => {}
        }
        out(cb, pk, format!("You equip {}.", item.name), OutputStyle::Loot);
    }

    fn use_item(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, item_name: &str, cb: &mut CommandCallbacks) {
        if item_name.is_empty() {
            out(cb, pk, "Use what? Usage: use <item>", OutputStyle::System);
            return;
        }
        let item_id = find_item_in_inventory(players.get(pk).unwrap(), item_name);
        let Some(item_id) = item_id else {
            out(cb, pk, format!("You don't have \"{item_name}\"."), OutputStyle::System);
            return;
        };
        let player = players.get_mut(pk).unwrap();
        if item_id == "mana_potion" {
            player.remove_item(&item_id);
            let restored = (40).min(player.data.max_mp - player.data.mp);
            player.data.mp += restored;
            out(cb, pk, format!("You drink Mana Potion and recover {restored} MP."), OutputStyle::Loot);
        } else if let Some(item) = ITEMS.get(&item_id) {
            if item.item_type == "consumable" {
                if let Some(heal) = item.heal {
                    player.remove_item(&item_id);
                    let healed = heal.min(player.data.max_hp - player.data.hp);
                    player.data.hp += healed;
                    out(cb, pk, format!("You drink {} and recover {healed} HP.", item.name), OutputStyle::Loot);
                } else {
                    out(cb, pk, format!("You can't use {} right now.", item.name), OutputStyle::System);
                    return;
                }
            } else {
                out(cb, pk, format!("You can't use {} right now.", item.name), OutputStyle::System);
                return;
            }
        }
        if let Some(p) = players.get(pk) {
            (cb.send)(pk, ServerMessage::Stats { player: p.to_snapshot(&self.world) });
        }
    }

    fn show_stats(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let player = players.get(pk).unwrap();
        let cls = class_stats(player.data.class_name);
        let text = format!(
            "-- {} --\n{} | Level {}\nHP: {}/{}  MP: {}/{}\nXP: {}/{}\nAttack: {}  Defense: {}\nGold: {}\nAbility: {} ({} MP)",
            player.username(), cls.display_name, player.data.level,
            player.data.hp, player.data.max_hp, player.data.mp, player.data.max_mp,
            player.data.xp, player.xp_to_level(),
            player.total_attack(), player.total_defense(), player.data.gold,
            cls.ability, cls.ability_cost,
        );
        out(cb, pk, text, OutputStyle::System);
        (cb.send)(pk, ServerMessage::Stats { player: player.to_snapshot(&self.world) });
    }

    fn show_quests(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let player = players.get(pk).unwrap();
        let active = player.get_active_quests();
        let complete: Vec<_> = player.data.quests.iter().filter(|q| q.status == QuestStatus::Complete).collect();
        if active.is_empty() && complete.is_empty() {
            out(cb, pk, "You have no quests. Talk to NPCs to find work.", OutputStyle::Quest);
            return;
        }
        let mut lines = Vec::new();
        let quests = self.world.quests();
        for qp in active {
            if let Some(q) = quests.get(&qp.quest_id) {
                lines.push(format_quest_progress(q, qp));
            }
        }
        for qp in complete {
            if let Some(q) = quests.get(&qp.quest_id) {
                lines.push(format!("[{}] COMPLETE - return to quest giver!", q.name));
            }
        }
        out(cb, pk, lines.join("\n\n"), OutputStyle::Quest);
    }

    fn talk(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, npc_name: &str, cb: &mut CommandCallbacks) {
        if npc_name.is_empty() {
            out(cb, pk, "Talk to whom? Usage: talk <npc>", OutputStyle::System);
            return;
        }
        let room_id = players.get(pk).unwrap().room_id().to_string();
        let lower = npc_name.to_lowercase();
        let npcs = self.world.npcs();
        let npc_id = self.world.get_room(&room_id).unwrap().npcs.iter().find(|id| {
            npcs.get(*id).map(|n| n.name.to_lowercase().contains(&lower) || id.to_lowercase().contains(&lower)).unwrap_or(false)
        }).cloned();
        let Some(npc_id) = npc_id else {
            out(cb, pk, format!("No \"{npc_name}\" here."), OutputStyle::System);
            return;
        };
        let npc = npcs.get(&npc_id).unwrap();
        out(cb, pk, format!("{}: \"{}\"", npc.name, npc.greeting), OutputStyle::Chat);
        if let Some(quest_id) = &npc.quest_id {
            let quests = self.world.quests();
            if let Some(quest) = quests.get(quest_id) {
                let player = players.get(pk).unwrap();
                if !player.data.quests.iter().any(|q| q.quest_id == *quest_id) {
                    out(cb, pk, format!("[Quest available: {}] Type 'accept {quest_id}' to begin.", quest.name), OutputStyle::Quest);
                }
            }
        }
    }

    fn accept_quest(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, quest_id: &str, cb: &mut CommandCallbacks) {
        if quest_id.is_empty() {
            out(cb, pk, "Accept which quest? Usage: accept <quest_id>", OutputStyle::System);
            return;
        }
        let quests = self.world.quests();
        let Some(quest) = quests.get(quest_id) else {
            out(cb, pk, format!("Unknown quest: {quest_id}"), OutputStyle::System);
            return;
        };
        let quest = quest.clone();
        let player = players.get_mut(pk).unwrap();
        if let Some(existing) = player.data.quests.iter().find(|q| q.quest_id == quest_id) {
            if existing.status != QuestStatus::TurnedIn {
                out(cb, pk, "You already have that quest.", OutputStyle::Quest);
                return;
            }
        }
        let room = self.world.get_room(player.room_id()).unwrap();
        if !room.npcs.contains(&quest.giver_npc) {
            out(cb, pk, "You must be near the quest giver to accept this quest.", OutputStyle::Quest);
            return;
        }
        if let Some(existing) = player.data.quests.iter_mut().find(|q| q.quest_id == quest_id) {
            existing.status = QuestStatus::Active;
            existing.progress = create_quest_progress(&quest).progress;
        } else {
            player.data.quests.push(create_quest_progress(&quest));
        }
        out(cb, pk, format!("Quest accepted: {}", quest.name), OutputStyle::Quest);
        out(cb, pk, quest.description.clone(), OutputStyle::Quest);
    }

    fn complete_quest(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, quest_id: &str, cb: &mut CommandCallbacks) {
        let quest_id_owned = if quest_id.is_empty() {
            let completable: Vec<String> = players.get(pk).unwrap().data.quests.iter()
                .filter(|q| q.status == QuestStatus::Complete)
                .map(|q| q.quest_id.clone()).collect();
            if completable.len() == 1 {
                completable[0].clone()
            } else {
                out(cb, pk, "Turn in which quest? Usage: complete <quest_id>", OutputStyle::System);
                return;
            }
        } else {
            quest_id.to_string()
        };
        let player = players.get_mut(pk).unwrap();
        let qp = player.data.quests.iter().find(|q| q.quest_id == quest_id_owned);
        if qp.map(|q| q.status) != Some(QuestStatus::Complete) {
            out(cb, pk, "That quest is not ready to turn in.", OutputStyle::Quest);
            return;
        }
        let quest = self.world.quests().get(&quest_id_owned).unwrap().clone();
        let room = self.world.get_room(player.room_id()).unwrap();
        if !room.npcs.contains(&quest.giver_npc) {
            out(cb, pk, "You must return to the quest giver.", OutputStyle::Quest);
            return;
        }
        for obj in &quest.objectives {
            if obj.objective_type == "collect" {
                for _ in 0..obj.count {
                    player.remove_item(&obj.target);
                }
            }
        }
        if let Some(qp) = player.data.quests.iter_mut().find(|q| q.quest_id == quest_id_owned) {
            qp.status = QuestStatus::TurnedIn;
        }
        player.data.gold += quest.rewards.gold;
        let xp_msgs = player.add_xp(quest.rewards.xp);
        if !quest.rewards.items.is_empty() {
            for item_id in &quest.rewards.items {
                player.add_item(item_id);
                let name = ITEMS.get(item_id).map(|i| i.name.clone()).unwrap_or_else(|| item_id.clone());
                out(cb, pk, format!("Received: {name}"), OutputStyle::Loot);
            }
        }
        out(cb, pk, format!("Quest turned in: {}! +{} gold, +{} XP", quest.name, quest.rewards.gold, quest.rewards.xp), OutputStyle::Quest);
        for msg in xp_msgs { out(cb, pk, msg, OutputStyle::Quest); }
        if let Some(p) = players.get(pk) {
            (cb.send)(pk, ServerMessage::Stats { player: p.to_snapshot(&self.world) });
        }
    }

    fn buy(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, item_name: &str, cb: &mut CommandCallbacks) {
        if item_name.is_empty() {
            out(cb, pk, "Buy what? Usage: buy <item>", OutputStyle::System);
            return;
        }
        let room_id = players.get(pk).unwrap().room_id().to_string();
        let lower = item_name.to_lowercase().replace(' ', "_");
        let npcs = self.world.npcs();
        let room = self.world.get_room(&room_id).unwrap();
        for npc_id in &room.npcs {
            let Some(npc) = npcs.get(npc_id) else { continue };
            if npc.shop.is_empty() { continue; }
            for listing in &npc.shop {
                let item = ITEMS.get(&listing.item_id);
                let name_match = item.map(|i| i.name.to_lowercase().replace(' ', "_").contains(&lower)).unwrap_or(false);
                if listing.item_id == lower || name_match {
                    let player = players.get_mut(pk).unwrap();
                    if player.data.gold < listing.price {
                        let name = item.map(|i| i.name.as_str()).unwrap_or(&listing.item_id);
                        out(cb, pk, format!("Not enough gold. {name} costs {} gold.", listing.price), OutputStyle::System);
                        return;
                    }
                    player.data.gold -= listing.price;
                    player.add_item(&listing.item_id);
                    let name = item.map(|i| i.name.clone()).unwrap_or(listing.item_id.clone());
                    out(cb, pk, format!("You buy {name} for {} gold.", listing.price), OutputStyle::Loot);
                    if let Some(p) = players.get(pk) {
                        (cb.send)(pk, ServerMessage::Stats { player: p.to_snapshot(&self.world) });
                    }
                    return;
                }
            }
        }
        out(cb, pk, format!("Nobody here sells \"{item_name}\"."), OutputStyle::System);
    }

    fn rest(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let player = players.get_mut(pk).unwrap();
        if player.room_id() != "eldermoor_tavern" {
            out(cb, pk, "You can only rest at The Gilded Tankard tavern.", OutputStyle::System);
            return;
        }
        if player.in_combat() {
            out(cb, pk, "You cannot rest during combat!", OutputStyle::Combat);
            return;
        }
        let healed = 30.min(player.data.max_hp - player.data.hp);
        let mp = 20.min(player.data.max_mp - player.data.mp);
        player.data.hp += healed;
        player.data.mp += mp;
        out(cb, pk, format!("You rest by the hearth. Recovered {healed} HP and {mp} MP."), OutputStyle::System);
        (cb.send)(pk, ServerMessage::Stats { player: player.to_snapshot(&self.world) });
    }

    fn help(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let cls = class_stats(players.get(pk).unwrap().data.class_name);
        let text = format!(
            "=== REALM OF ECHOES - Commands ===\n\nMovement:     north/south/east/west/up/down (n/s/e/w/u/d)\nLook:         look (l)\nCombat:       attack <target>, ability, special (Lv.{})\nItems:        get/take, drop, inventory (i), equip, use, buy, craft\nSocial:       say/yell/whisper/global/emote, party, guild, trade\nQuests:       talk <npc>, accept <quest_id>, complete <quest_id>\nInfo:         stats, who, help, rest, quit\nHotkeys:      n/s/e/w move, l look, i inv, h help\n\nAbilities:    {} ({} MP)\n              {} ({} MP, Lv.{})",
            cls.ability2_level, cls.ability, cls.ability_cost, cls.ability2, cls.ability2_cost, cls.ability2_level,
        );
        out(cb, pk, text, OutputStyle::System);
    }

    fn global(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, message: &str, cb: &mut CommandCallbacks) {
        if message.is_empty() {
            out(cb, pk, "Global what? Usage: global <message>", OutputStyle::System);
            return;
        }
        let username = players.get(pk).unwrap().username().to_string();
        (cb.global_broadcast)(&format!("[Global] {username}: {message}"), OutputStyle::Global);
    }

    fn emote(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, action: &str, cb: &mut CommandCallbacks) {
        if action.is_empty() {
            out(cb, pk, "Emote what? Usage: emote <action>", OutputStyle::System);
            return;
        }
        let (room_id, username) = {
            let p = players.get(pk).unwrap();
            (p.room_id().to_string(), p.username().to_string())
        };
        for (key, p) in players.iter() {
            if p.room_id() == room_id {
                out(cb, key, format!("* {username} {action}"), OutputStyle::Emote);
            }
        }
    }

    fn search(&self, pk: &str, players: &mut HashMap<String, PlayerSession>, cb: &mut CommandCallbacks) {
        let room_id = players.get(pk).unwrap().room_id().to_string();
        let hint = match room_id.as_str() {
            "abandoned_shrine" => Some("You examine the iron door. It bears a keyhole shaped like a serpent. Perhaps the Ancient Key fits."),
            "eldermoor_tavern" => Some("You kick over a floorboard and find 5 gold tucked beneath!"),
            "moonlit_clearing" => Some("Strange runes on the stones pulse faintly. The woods feel watchful."),
            "crypt_tomb" => Some("The Lich King's throne is carved from a single piece of obsidian."),
            _ => None,
        };
        if let Some(h) = hint {
            out(cb, pk, h, OutputStyle::System);
            let player = players.get_mut(pk).unwrap();
            if room_id.as_str() == "eldermoor_tavern" && !player.searched_rooms.contains("eldermoor_tavern") {
                player.searched_rooms.insert("eldermoor_tavern".into());
                player.data.gold += 5;
                out(cb, pk, "You found 5 gold!", OutputStyle::Loot);
            }
        } else {
            out(cb, pk, "You find nothing unusual.", OutputStyle::System);
        }
    }

    fn leaderboard(&self, pk: &str, cb: &mut CommandCallbacks) {
        let mut all = get_all_players();
        all.sort_by(|a, b| b.level.cmp(&a.level).then(b.kills.cmp(&a.kills)));
        let mut lines = vec!["-- Leaderboard (Level) --".to_string()];
        for (i, p) in all.iter().take(10).enumerate() {
            lines.push(format!("  {}. {} — Lv.{} ({} kills)", i + 1, p.username, p.level, p.kills));
        }
        out(cb, pk, lines.join("\n"), OutputStyle::System);
    }

    fn party_peers_in_room(&self, pk: &str, players: &HashMap<String, PlayerSession>) -> Vec<String> {
        let Some(player) = players.get(pk) else { return vec![] };
        let room = player.room_id().to_string();
        self.party
            .get_party_peers(player, players)
            .into_iter()
            .filter(|p| p.username().to_lowercase() != pk && p.room_id() == room)
            .map(|p| p.username().to_lowercase())
            .collect()
    }

    fn find_player_in_room(&self, pk: &str, players: &HashMap<String, PlayerSession>, name: &str) -> Option<String> {
        let lower = name.to_lowercase();
        let room = players.get(pk)?.room_id();
        for (key, p) in players {
            if key == pk || !p.authenticated || p.room_id() != room { continue; }
            if p.username().to_lowercase().contains(&lower) {
                return Some(key.clone());
            }
        }
        None
    }

    fn send_combat_snapshot(
        &self,
        pk: &str,
        players: &HashMap<String, PlayerSession>,
        cb: &mut CommandCallbacks,
    ) {
        let Some(player) = players.get(pk) else {
            return;
        };

        let state = if let Some(instance_id) = &player.combat_target {
            let room_id = player.room_id();
            self.world.get_room(room_id).and_then(|room| {
                let mob = room.mobs.iter().find(|m| m.instance_id == *instance_id)?;
                let mobs = self.world.mobs();
                let tmpl = mobs.get(&mob.template_id)?;
                Some(CombatSnapshot {
                    in_combat: true,
                    target: Some(tmpl.name.clone()),
                    target_hp: Some(mob.hp),
                    target_max_hp: Some(mob.max_hp),
                })
            })
        } else if let Some(opp_key) = &player.pvp_target {
            players.get(opp_key).map(|opp| CombatSnapshot {
                in_combat: true,
                target: Some(opp.username().to_string()),
                target_hp: Some(opp.data.hp),
                target_max_hp: Some(opp.data.max_hp),
            })
        } else {
            None
        };

        let state = state.unwrap_or(CombatSnapshot {
            in_combat: false,
            target: None,
            target_hp: None,
            target_max_hp: None,
        });

        (cb.send)(pk, ServerMessage::Combat { state });
    }
}

fn out(cb: &mut CommandCallbacks, pk: &str, text: impl Into<String>, style: OutputStyle) {
    (cb.send)(pk, ServerMessage::Output { text: text.into(), style: Some(style) });
}

fn opposite_dir(dir: &str) -> String {
    match dir {
        "north" => "south".into(),
        "south" => "north".into(),
        "east" => "west".into(),
        "west" => "east".into(),
        "up" => "down".into(),
        "down" => "up".into(),
        other => other.into(),
    }
}