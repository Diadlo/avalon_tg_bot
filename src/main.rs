mod game;

use std::{sync::Arc, ops::{Deref, DerefMut}, collections::HashMap, error::Error};

use game::{Game, GameEvent};
use teloxide::prelude::*;
use tokio::sync::Mutex;

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
    user_names: HashMap<ChatId, String>,
}

struct GameInfo {
    players: Vec<ChatId>,
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

async fn handle_exit<'a, I>(bot: &Bot, message: &Message, ctx: &mut BotCtx, mut cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
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

async fn handle_new_game<'a, I>(bot: &Bot, message: &Message, ctx: &mut BotCtx, cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
{
    let game_id = ctx.last_game_id + 1;
    let session = GameSession {
        id: game_id,
        leader: message.chat.id,
        user_names: HashMap::new(),
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

    respond(())
}

fn send_everybody(bot: &Bot, session: &GameSession, msg: &str) {
    let info = session.info.unwrap();
    for player in &info.players {
        let name = session.user_names.get(player).unwrap();
        bot.send_message(*player, msg);
    }
}

async fn process_game_event(bot: &Bot, session: &mut GameSession) -> Result<(), Box<dyn Error>>
{
    let info = session.info.as_mut().unwrap();
    let event = info.cli.lock().await.recv_event().await.unwrap();
    match event {
        GameEvent::Turn(crown_id, team_size) => {
            let crown_name = session.user_names.get(&info.players[crown_id as usize]).unwrap();
            let message = format!("{} proposes a team of {} people", crown_name, team_size);
            send_everybody(&bot, &session, message.as_str());
        },
        GameEvent::TeamSuggested(team) => {
            let message = format!("Suggested team: {}", team.iter().map(|id| {
                let chat_id = info.players[*id as usize];
                session.user_names.get(&chat_id).unwrap().clone()
            }).collect::<Vec<_>>().join(", "));
            send_everybody(&bot, &session, message.as_str());
            send_everybody(&bot, &session, "Use /approve or /reject to vote");
        },
        GameEvent::TeamVote(votes) => {
            let message = format!("Votes: \n{}", info.players.iter().zip(votes).map(|vote| {
                let chat_id = vote.0;
                let name = session.user_names.get(&chat_id).unwrap();
                format!("{} - {}", name, vote.1)
            }).collect::<Vec<_>>().join("\n"));
            send_everybody(&bot, &session, message.as_str());
        },
        GameEvent::TeamApproved(team) => {
            send_everybody(&bot, &session, "Team approved");
            for player in &team {
                let chat_id = session.info.as_mut().unwrap().players[*player as usize];
                bot.send_message(chat_id, format!("You are on the mission team. Select the result /success or /fail"));
            }
        },
        GameEvent::TeamRejected(try_count) => {
            send_everybody(&bot, &session, format!("Team rejected. Try count: {}/{}", try_count, game::MAX_TRY_COUNT).as_str());
        },
        GameEvent::MissionResult(results) => {
            let message = format!("Mission results: {}", results.iter().map(|result| {
                result.to_string()
            }).collect::<Vec<_>>().join(", "));
            send_everybody(&bot, &session, message.as_str());
        },
        GameEvent::Mermaid(mermaid_id) => {
            let mermaid_chat_id = info.players[mermaid_id as usize];
            let mermaid_name = session.user_names.get(&mermaid_chat_id).unwrap();
            let message = format!("{} is going to use mermaid", mermaid_name);
            send_everybody(&bot, &session, message.as_str());

            let users = session.info.as_mut().unwrap().players.iter()
                .filter(|id| **id != mermaid_chat_id)
                .enumerate()
                .map(|(id, chat_id)| {
                    let username = session.user_names.get(&chat_id).unwrap().clone();
                    format!("{}. {}", id, username)
                })
                .collect::<Vec<_>>()
                .join("\n");
            let message = format!("Use mermaid. Enter /mermaid <id> to check user:\n{}", users);
            bot.send_message(mermaid_chat_id, message).await?;
        },
        GameEvent::MermaidResult(user, team) => {
            let mermaid_id = info.cli.lock().await.get_mermaid_id().await;
            let mermaid_chat_id = info.players[mermaid_id as usize];

            let checked_user_id = info.players[user as usize];
            let checked_user_name = session.user_names.get(&checked_user_id).unwrap();

            let message = format!("{} is {}", checked_user_name, team);

            bot.send_message(mermaid_chat_id, message);
            bot.send_message(mermaid_chat_id, "Select want you want to announce /good or /bad");
        },
        GameEvent::MermaidSays(user, team) => {
            let checked_user_id = info.players[user as usize];
            let checked_user_name = session.user_names.get(&checked_user_id).unwrap();
            let message = format!("Mermaid says {} is {}", checked_user_name, team);
            send_everybody(bot, &session, &message);
        },
        GameEvent::BadLastChance(bad_team, guesser) => {
            send_everybody(&bot, &session, "Good team are winning, but bad team has one last chance");

            let message = format!("Bad team: {}", bad_team.iter().map(|id| {
                let chat_id = session.info.as_mut().unwrap().players[*id as usize];
                session.user_names.get(&chat_id).unwrap().clone()
            }).collect::<Vec<_>>().join(", "));
            send_everybody(&bot, &session, message.as_str());

            let guesser_chat_id = session.info.as_mut().unwrap().players[guesser as usize];
            let guesser_name = session.user_names.get(&guesser_chat_id).unwrap();
            let message = format!("{} is going to guess Merlin", guesser_name);
            send_everybody(&bot, &session, message.as_str());

            let good_team = session.info.as_mut().unwrap().players.iter()
                .enumerate()
                .filter(|(id, chat_id)| {
                    !bad_team.contains(&(*id as u8))
                });

            let users = good_team
                .map(|(id, chat_id)| {
                    let username = session.user_names.get(&chat_id).unwrap().clone();
                    format!("{}. {}", id, username)
                })
                .collect::<Vec<_>>()
                .join("\n");
            let message = format!("Enter /merlin <id> to check user:\n{}", users);
            bot.send_message(guesser_chat_id, message).await?;
        },
        GameEvent::Merlin(merlin_id) => {
            let merlin_chat_id = info.players[merlin_id as usize];
            let merlin_name = session.user_names.get(&merlin_chat_id).unwrap();
            let message = format!("{} is Merlin", merlin_name);
            send_everybody(&bot, &session, message.as_str());
        },
        GameEvent::GameResult(result) => {
            if result == game::GameResult::GoodWins {
                send_everybody(&bot, &session, "Good team won!");
            } else {
                send_everybody(&bot, &session, "Bad team won!");
            }
        },
    }

    Ok(())
}

async fn handle_start_game<'a, I>(bot: &Bot, message: &Message, ctx: &mut BotCtx, cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
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

            let mut info = GameInfo {
                players,
                cli,
            };

            // fill session user names
            for player in &info.players {
                let name = ctx.user_names.get(player).unwrap();
                session.user_names.insert(*player, name.clone());
            }

            tokio::spawn(async move {
                if let Err(e) = game.start().await {
                    println!("Game error: {}", e);
                }
            });

            session.info = Some(info);

            let bot = bot.clone();
            tokio::spawn(async move {
                loop {
                    process_game_event(bot, session).await;
                }
            });
        } else {
            bot.send_message(message.chat.id, "Only game leader can start the game").await?;
        }
    } else {
        bot.send_message(message.chat.id, "You are not in a game. Join or create new one").await?;
    }

    respond(())
}

async fn handle_tg_message(bot: Bot, message: Message, ctx: Arc<Mutex<BotCtx>>) -> ResponseResult<()>
{
    if let Some(text) = message.text() {
        let mut ctx = ctx.lock().await;
        let mut cmd = text.split_whitespace();
        match cmd.next().unwrap() {
            "/start" => {
                return handle_start_bot(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }
            "/new_game" => {
                return handle_new_game(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }
            "/start_game" => {
                return handle_start_game(&bot, &message, &mut ctx.deref_mut(), cmd).await;
            }
            "/exit" => {
                return handle_exit(&bot, &message, &mut ctx.deref_mut(), cmd).await;
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
        game_sessions: HashMap::new()
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
