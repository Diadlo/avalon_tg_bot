mod game;
mod game_msg;

use std::{sync::Arc, ops::{Deref, DerefMut}, collections::HashMap, error::Error};
use std::fmt::format;

use game::{Game, GameEvent};
use game_msg::GameMessage;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use crate::game::Role::Percival;
use crate::game::{MissionVote, Team, TeamVote};

const BOT_TG_ADDR: &str = "the_resistance_avalon_bot";

struct BotCtx {
    last_game_id: u32,
    user_names: HashMap<ChatId, String>,
    user_games: HashMap<ChatId, u32>,
    game_sessions: HashMap<u32, GameSession>
}

struct GameSession {
    id: u32,
    leader: ChatId,
    info: Option<GameInfo>,
}

// TODO: Move out to separate file
#[derive(Clone)]
pub struct GameInfo {
    players: Vec<ChatId>,
    user_names: HashMap<ChatId, String>,
    cli: Arc<Mutex<game::GameClient>>,
}

async fn handle_start_bot<'a, I>(bot: &Bot, message: &Message, ctx: &mut BotCtx, mut cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
{
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        // If there is entry in user_games, game entry must exist
        let _ = ctx.game_sessions.get(game_id).unwrap();
        bot.send_message(message.chat.id, "You are already in the game").await?;
        bot.send_message(message.chat.id, "If you want to leave it, use /exit command, than join the link again").await?;
    } else {
        if let Some(param) = cmd.next() {
            if let Ok(game_id) = param.parse::<u32>() {
                if let Some(info) = ctx.game_sessions.get_mut(&game_id) {
                    bot.send_message(message.chat.id, "You are joined the game").await?;
                    bot.send_message(message.chat.id, "Wait for the game to start").await?;
                    let name = if let Some(user) = &message.from() {
                        user.first_name.clone()
                    } else {
                        message.chat.id.to_string()
                    };

                    bot.send_message(info.leader, format!("{} joined the game", name)).await?;
                    ctx.user_games.insert(message.chat.id, game_id);
                    ctx.user_names.insert(message.chat.id, name);
                } else {
                    bot.send_message(message.chat.id, "Invalid game id!").await?;
                }
            } else {
                bot.send_message(message.chat.id, "Invalid game id!").await?;
            }
        } else {
            bot.send_message(message.chat.id, "Welcome to The Resistance Avalon Bot!").await?;
            bot.send_message(message.chat.id, "Use /new_game command to create game session").await?;
            bot.send_message(message.chat.id, "Or join existing game using invite link").await?;
        }
    }

    respond(())
}

async fn handle_exit(bot: &Bot, message: &Message, ctx: &mut BotCtx) -> ResponseResult<()>
{
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        if let Some(info) = ctx.game_sessions.get(game_id) {
            bot.send_message(message.chat.id, "You are left the game").await?;
            bot.send_message(info.leader, format!("{} left the game", message.chat.id)).await?;
            ctx.user_games.remove(&message.chat.id);
        }
    } else {
        bot.send_message(message.chat.id, "You are not in the game").await?;
    }

    respond(())
}

async fn handle_new_game(bot: &Bot, message: &Message, ctx: &mut BotCtx) -> ResponseResult<()>
{
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        // If there is entry in user_games, game entry must exist
        let _ = ctx.game_sessions.get(game_id).unwrap();
        bot.send_message(message.chat.id, "You are already in the game").await?;
        bot.send_message(message.chat.id, "If you want to leave it, use /exit command, than join the link again").await?;
    } else {
        let game_id = ctx.last_game_id + 1;
        let session = GameSession {
            id: game_id,
            leader: message.chat.id,
            info: None,
        };

        ctx.game_sessions.insert(session.id, session);
        ctx.user_games.insert(message.chat.id, game_id);
        ctx.last_game_id += 1;

        let id = message.chat.id;
        bot.send_message(id, "Starting a new game...").await?;
        bot.send_message(id, "Send the following invite link to your team").await?;
        let url = format!("https://t.me/{}?start={}", BOT_TG_ADDR, game_id);
        bot.send_message(id, url).await?;
    }

    respond(())
}

fn send_everybody(bot: &Bot, info: &GameInfo, msg: &str) {
    for player in &info.players {
        let name = info.user_names.get(player).unwrap();
        let _ = bot.send_message(*player, msg);
    }
}

async fn send_not_in_game(bot: &Bot, message: &Message) -> ResponseResult<()> {
    bot.send_message(message.chat.id, "You are not in a game. Join or create new one").await?;
    respond(())
}

async fn process_game_event(bot: &Bot, info: &GameInfo) -> Result<(), Box<dyn Error>>
{
    let event = info.cli.lock().await.recv_event().await.unwrap();
    let messages = game_msg::build_message_for_event(info, event).await?;

    let send_message = for msg in messages {
        match msg {
            GameMessage::Notification(notification) => {
                match notification.dst {
                    game_msg::Dst::All => {
                        send_everybody(bot, info, &notification.message);
                        None
                    }
                    game_msg::Dst::User(id) => {
                        let res = bot.send_message(id, &notification.message).await?;
                        Some(res)
                    }
                }
            }
            GameMessage::ControlMessage(control) => {
                let message = format!("{}:\n{}", control.message, control.commands.join("\n"));
                match control.dst {
                    game_msg::Dst::All => {
                        send_everybody(bot, info, message.as_str());
                        None
                    }
                    game_msg::Dst::User(id) => {
                        let res = bot.send_message(id, message).await?;
                        Some(res)
                    }
                }
            }
        }
    };

    if let Some(send_message) = send_message {
        let suggestion_message_id = send_message.payload.reply_to_message_id;
    }

    Ok(())
}

async fn handle_start_game(bot: &Bot, message: &Message, ctx: &mut BotCtx) -> ResponseResult<()>
{
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        // If there is entry in user_games, game entry must exist
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        if session.leader == message.chat.id {
            let players = ctx.user_games.iter()
                .filter(|entry| { entry.1 == game_id })
                .map(|entry| { entry.0.clone() })
                .collect::<Vec<_>>();

            for player in &players {
                bot.send_message(*player, "Game started!").await?;
            }

            let (mut game, cli) = game::Game::setup(players.len());

            let roles = cli.get_player_roles().await;
            for (player, role) in players.iter().zip(roles) {
                bot.send_message(*player, format!("Your role is {}", role.to_string())).await?;
            }

            let crown_id = cli.get_crown_id().await;
            let crown_chat_id = players[crown_id as usize];
            let crown_name = ctx.user_names.get(&crown_chat_id).unwrap();

            let mermaid_id = cli.get_mermaid_id().await;
            let mermaid_chat_id = players[mermaid_id as usize];
            let mermaid_name = ctx.user_names.get(&mermaid_chat_id).unwrap();

            for player in &players {
                let crown_name = if *player == crown_chat_id { "You" } else { crown_name };
                let mermaid_name = if *player == mermaid_chat_id { "You" } else { mermaid_name };

                bot.send_message(*player, format!("{} has the crown", crown_name)).await?;
                bot.send_message(*player, format!("{} is the mermaid", mermaid_name)).await?;
            }

            let user_names = {
                let mut user_names = HashMap::new();
                for player in &players {
                    let name = ctx.user_names.get(player).unwrap();
                    user_names.insert(*player, name.clone());
                }
                user_names
            };

            let info = GameInfo {
                players,
                cli: Arc::new(Mutex::new(cli)),
                user_names,
            };

            session.info = Some(info.clone());

            tokio::spawn(async move {
                if let Err(e) = game.start().await {
                    println!("Game error: {}", e);
                }
            });

            let bot = bot.clone();
            tokio::spawn(async move {
                let info = info.clone();
                loop {
                    if let Err(e) = process_game_event(&bot, &info).await {
                        println!("Event processing error: {}", e);
                        break;
                    }
                }
            });
        } else {
            bot.send_message(message.chat.id, "Only game leader can start the game").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_team_suggestion(bot: &Bot, message: &Message, ctx: &mut &mut BotCtx, cmd: &str) -> ResponseResult<()> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.lock().await;
        let user_id = info.players.iter().position(|&id| { id == message.chat.id }).unwrap() as u8;
        let vote_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if todo!() {
        } else {
            bot.send_message(message.chat.id, "Invalid vote command").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_team_vote(bot: &Bot, message: &Message, ctx: &mut &mut BotCtx, cmd: &str) -> ResponseResult<()> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.lock().await;
        let user_id = info.players.iter().position(|&id| { id == message.chat.id }).unwrap() as u8;
        let vote_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(vote) = vote_cmd.get(1) {
            match *vote {
                "approve" => {
                    cli.add_team_vote(user_id, TeamVote::Approve).await.unwrap();
                },
                "reject" => {
                    cli.add_team_vote(user_id, TeamVote::Reject).await.unwrap();
                },
                _ => {
                    bot.send_message(message.chat.id, "Invalid vote command").await?;
                }
            }
        } else {
            bot.send_message(message.chat.id, "Invalid vote command").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_mission_result(bot: &Bot, message: &Message, ctx: &mut &mut BotCtx, cmd: &str) -> ResponseResult<()> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.lock().await;
        let user_id = info.players.iter().position(|&id| { id == message.chat.id }).unwrap() as u8;
        let result_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(vote) = result_cmd.get(1) {
            match *vote {
                "success" => {
                    cli.submit_for_mission(user_id, MissionVote::Success).await.unwrap();
                },
                "fail" => {
                    cli.submit_for_mission(user_id, MissionVote::Fail).await.unwrap();
                },
                _ => {
                    bot.send_message(message.chat.id, "Invalid result command").await?;
                }
            }
        } else {
            bot.send_message(message.chat.id, "Invalid result command").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_mermaid(bot: &Bot, message: &Message, ctx: &mut &mut BotCtx, cmd: &str) -> ResponseResult<()> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.lock().await;
        let mermaid_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(check_id) = mermaid_cmd.get(1) {
            if let Some(check_id) = check_id.parse::<u8>().ok() {
                cli.send_mermaid_selection(check_id).await.unwrap();
            } else {
                bot.send_message(message.chat.id, "Invalid mermaid command").await?;
            }
        } else {
            bot.send_message(message.chat.id, "Invalid mermaid command").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_mermaid_word(bot: &Bot, message: &Message, ctx: &mut &mut BotCtx, cmd: &str) -> ResponseResult<()> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.lock().await;
        let mermaid_word = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(word) = mermaid_word.get(1) {
            match *word {
                "good" => {
                    cli.send_mermaid_word(Team::Good).await.unwrap();
                },
                "bad" => {
                    cli.send_mermaid_word(Team::Bad).await.unwrap();
                },
                _ => {
                    bot.send_message(message.chat.id, "Invalid mermaid word").await?;
                }
            }
        } else {
            bot.send_message(message.chat.id, "Invalid mermaid word").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_last_chance(bot: &Bot, message: &Message, ctx: &mut &mut BotCtx, cmd: &str) -> ResponseResult<()> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        let session = ctx.game_sessions.get_mut(game_id).unwrap();
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.lock().await;
        let merlin_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(merlin_id) = merlin_cmd.get(1) {
            if let Some(merlin_id) = merlin_id.parse::<u8>().ok() {
                cli.send_merlin_check(merlin_id).await.unwrap();
            } else {
                bot.send_message(message.chat.id, "Invalid last chance command").await?;
            }
        } else {
            bot.send_message(message.chat.id, "Invalid last chance command").await?;
        }
    } else {
        send_not_in_game(bot, message).await?;
    }

    respond(())
}

async fn handle_tg_message(bot: Bot, message: Message, ctx: Arc<Mutex<BotCtx>>) -> ResponseResult<()>
{
    if let Some(text) = message.text() {
        let mut ctx = ctx.lock().await;
        let mut input = text.split_whitespace();
        let cmd = input.next().unwrap();
        let args = input;
        match cmd {
            "/start" => {
                return handle_start_bot(&bot, &message, &mut ctx.deref_mut(), args).await;
            }
            "/new_game" => {
                return handle_new_game(&bot, &message, &mut ctx.deref_mut()).await;
            }
            "/start_game" => {
                return handle_start_game(&bot, &message, &mut ctx.deref_mut()).await;
            }
            "/exit" => {
                return handle_exit(&bot, &message, &mut ctx.deref_mut()).await;
            }

            cmd if cmd.starts_with("/suggest") => {
                return handle_team_suggestion(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }

            cmd if cmd.starts_with("/team") => {
                return handle_team_vote(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }

            cmd if cmd.starts_with("/mission") => {
                return handle_mission_result(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }

            cmd if cmd.starts_with("/mermaid") => {
                return handle_mermaid(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }

            cmd if cmd.starts_with("/say") => {
                return handle_mermaid_word(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }

            cmd if cmd.starts_with("/merlin") => {
                return handle_last_chance(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }

            _ => {
                bot.send_message(message.chat.id, "Unknown command").await?;
                return respond(());
            }
        }
    }

    respond(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut g, _cli) = game::Game::setup(5);

    let game_fut = async {
        g.start().await.unwrap();
        println!("End of game future");
    };

    let mut ctx = Arc::new(Mutex::new(BotCtx {
        last_game_id: 0,
        user_games: HashMap::new(),
        game_sessions: HashMap::new(),
        user_names: HashMap::new(),
    }));
    let bot = Bot::from_env();
    let bot_fut = async {
        let ctx = ctx.clone();
        teloxide::repl(bot, move |bot: Bot, message: Message| {
            let ctx = ctx.clone();
            async move { handle_tg_message(bot, message, ctx).await }
        })
        .await;
        println!("End of bot future");
    };

    tokio::join!(game_fut, bot_fut);

    Ok(())
}
