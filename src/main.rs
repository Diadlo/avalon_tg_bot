mod game;
mod game_msg;

use std::{sync::Arc, ops::DerefMut, collections::HashMap, error::Error};

use game::GameEvent;
use game_msg::GameMessage;
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tokio::sync::Mutex;
use crate::game::{MissionVote, Team, TeamVote};

const BOT_TG_ADDR: &str = "the_resistance_avalon_bot";

struct BotCtx {
    bot: Bot,
    last_game_id: u32,
    user_names: HashMap<ChatId, String>,
    user_games: HashMap<ChatId, u32>,
    game_sessions: HashMap<u32, Arc<Mutex<GameSession>>>,
}

struct SuggestionInfo {
    msg_id: MessageId,
    crown_id: u8,
    team_size: usize,
    users: Vec<u8>,
}

struct GameSession {
    id: u32,
    leader: ChatId,
    info: Option<GameInfo>,
    suggestion: Option<SuggestionInfo>,
}

// TODO: Move out to separate file
#[derive(Clone)]
pub struct GameInfo {
    players: Vec<ChatId>,
    user_names: HashMap<ChatId, String>,
    cli: game::GameClient,
}

async fn get_game_session(ctx: &mut BotCtx, message: &Message) -> Option<Arc<Mutex<GameSession>>> {
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        if let Some(session) = ctx.game_sessions.get(game_id).cloned() {
            let session_id = session.lock().await.id;
            // ID is set to zero when game is finished
            if session_id == 0 {
                drop(session);
                ctx.game_sessions.remove(&session_id);
                None
            } else {
                Some(session)
            }
        } else {
            None
        }
    } else {
        None
    }
}

async fn handle_start_bot<'a, I>(ctx: &mut BotCtx, message: &Message, mut cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
{
    if let Some(_) = get_game_session(ctx, message).await {
        ctx.bot.send_message(message.chat.id, "You are already in the game").await?;
        ctx.bot.send_message(message.chat.id, "If you want to leave it, use /exit command, than join the link again").await?;
    } else {
        if let Some(param) = cmd.next() {
            if let Ok(game_id) = param.parse::<u32>() {
                println!("Game ID: {}", game_id);
                println!("Game sessions: {}",
                         ctx.game_sessions.iter()
                             .map(|(k, v)| { format!("{}", *k) })
                             .collect::<Vec<_>>()
                             .join(","));
                if let Some(session) = ctx.game_sessions.get(&game_id) {
                    let session = session.lock().await;
                    ctx.bot.send_message(message.chat.id, "You are joined the game. Wait for the game to start").await?;
                    let name = if let Some(user) = &message.from() {
                        user.first_name.clone()
                    } else {
                        message.chat.id.to_string()
                    };

                    ctx.bot.send_message(session.leader, format!("{} joined the game", name)).await?;
                    ctx.user_games.insert(message.chat.id, game_id);
                    ctx.user_names.insert(message.chat.id, name);
                } else {
                    ctx.bot.send_message(message.chat.id, "Invalid game id!").await?;
                }
            } else {
                ctx.bot.send_message(message.chat.id, "Invalid game id!").await?;
            }
        } else {
            ctx.bot.send_message(message.chat.id, "Welcome to The Resistance Avalon Bot!").await?;
            ctx.bot.send_message(message.chat.id, "Use /new_game command to create game session").await?;
            ctx.bot.send_message(message.chat.id, "Or join existing game using invite link").await?;
        }
    }

    respond(())
}

async fn handle_exit(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()>
{
    if let Some(session) = get_game_session(ctx, message).await {
        let session = session.lock().await;
        ctx.bot.send_message(message.chat.id, "You left the game").await?;
        let username = ctx.user_names.get(&message.chat.id).unwrap();
        ctx.bot.send_message(session.leader, format!("{} left the game", username)).await?;
        ctx.user_games.remove(&message.chat.id);
    } else {
        ctx.bot.send_message(message.chat.id, "You are not in the game").await?;
    }

    respond(())
}

async fn handle_new_game(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()>
{
    if let Some(_) = get_game_session(ctx, message).await {
        ctx.bot.send_message(message.chat.id, "You are already in the game").await?;
        ctx.bot.send_message(message.chat.id, "If you want to leave it, use /exit command, than join the link again").await?;
    } else {
        let game_id = ctx.last_game_id + 1;
        let session = GameSession {
            id: game_id,
            leader: message.chat.id,
            info: None,
            suggestion: None,
        };

        ctx.game_sessions.insert(session.id, Arc::new(Mutex::new(session)));
        ctx.user_games.insert(message.chat.id, game_id);
        ctx.last_game_id += 1;

        let name = if let Some(user) = &message.from() {
            user.first_name.clone()
        } else {
            message.chat.id.to_string()
        };

        ctx.user_names.insert(message.chat.id, name);

        let id = message.chat.id;
        ctx.bot.send_message(id, "Starting a new game...").await?;
        ctx.bot.send_message(id, "Send the following invite link to your team").await?;
        let url = format!("https://t.me/{}?start={}", BOT_TG_ADDR, game_id);
        ctx.bot.send_message(id, url).await?;
        ctx.bot.send_message(id, "When everybody is joined use /start_game").await?;
    }

    respond(())
}

async fn send_everybody(bot: &Bot, info: &GameInfo, msg: &str) {
    for player in &info.players {
        println!("Message '{}' to {}", msg, *player);
        let _ = bot.send_message(*player, msg).await;
    }
}

async fn send_not_in_game(bot: &Bot, message: &Message) -> ResponseResult<()> {
    bot.send_message(message.chat.id, "You are not in a game. Join or create new one").await?;
    respond(())
}

fn control_message_to_string(control: &game_msg::ControlMessage) -> String {
    let commands = control.commands
        .iter()
        .map(|c| format!("/{}", c))
        .collect::<Vec<_>>();

    format!("{}:\n{}", control.message, commands.join("\n"))
}

async fn process_game_event(session: &mut GameSession, event: &GameEvent, bot: &Bot, info: &GameInfo) -> Result<(), Box<dyn Error>>
{
    println!(">process_game_event");
    let messages = game_msg::build_message_for_event(info, event.clone()).await?;
    println!("messages: {:?}", messages);

    // TODO: Extract to function returning message id of control message (if any)
    for msg in messages {
        match msg {
            GameMessage::Notification(notification) => {
                match notification.dst {
                    game_msg::Dst::All => {
                        send_everybody(bot, info, &notification.message).await;
                    }
                    game_msg::Dst::User(id) => {
                        println!("Message '{}' to {}", notification.message, id);
                        bot.send_message(id, &notification.message).await?;
                    }
                }
            }
            GameMessage::ControlMessage(control) => {
                let message = control_message_to_string(&control);
                match control.dst {
                    game_msg::Dst::All => {
                        send_everybody(bot, info, message.as_str()).await;
                    }
                    game_msg::Dst::User(id) => {
                        println!("Message '{}' to {}", message, id);
                        let res = bot.send_message(id, message).await?;
                        if let GameEvent::Turn(crown_id, team_size) = event {
                            session.suggestion = Some(SuggestionInfo {
                                msg_id: res.id,
                                crown_id: *crown_id,
                                team_size: *team_size,
                                users: Vec::new(),
                            });
                        }
                    }
                }
            }
        }
    }

    if let GameEvent::GameResult(_) = event {
        session.id = 0;
    }

    println!("<process_game_event");
    Ok(())
}

async fn handle_start_game(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()>
{
    if let Some(session_arc) = get_game_session(ctx, message).await {
        let mut session = session_arc.lock().await;
        if session.leader == message.chat.id {
            let players = ctx.user_games.iter()
                .filter(|entry| { *entry.1 == session.id })
                .map(|entry| { entry.0.clone() })
                .collect::<Vec<_>>();

            let start_msg = format!("Game started with {} players!", players.len());
            for player in &players {
                ctx.bot.send_message(*player, &start_msg).await?;
            }

            let (mut game, cli) = game::Game::setup(players.len());

            let roles = cli.get_player_roles().await;
            for (player, role) in players.iter().zip(roles) {
                ctx.bot.send_message(*player, format!("Your role is {}", role.to_string())).await?;
            }

            let crown_id = cli.get_crown_id().await;
            println!("Start game crown_id: {}", crown_id);
            let crown_chat_id = players[crown_id as usize];
            let crown_name = ctx.user_names.get(&crown_chat_id).unwrap();

            let mermaid_id = cli.get_mermaid_id().await;
            println!("Start game mermaid_id: {}", crown_id);
            let mermaid_chat_id = players[mermaid_id as usize];
            let mermaid_name = ctx.user_names.get(&mermaid_chat_id).unwrap();

            for player in &players {
                let crown_name = if *player == crown_chat_id { "You" } else { crown_name };
                let mermaid_name = if *player == mermaid_chat_id { "You" } else { mermaid_name };

                ctx.bot.send_message(*player, format!("{} has the crown", crown_name)).await?;
                ctx.bot.send_message(*player, format!("{} has the mermaid", mermaid_name)).await?;
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
                cli: cli.clone(),
                user_names,
            };

            session.info = Some(info.clone());
            drop(session);

            tokio::spawn(async move {
                if let Err(e) = game.start().await {
                    println!("Game error: {}", e);
                }
            });

            let bot = ctx.bot.clone();
            tokio::spawn(async move {
                let info = info.clone();
                let session = session_arc.clone();
                loop {
                    println!("Event processing iteration");
                    let event = info.cli.clone().recv_event().await.unwrap();
                    let mut session = session.lock().await;
                    if let Err(e) = process_game_event(session.deref_mut(), &event, &bot, &info).await {
                        println!("Event processing error: {}", e);
                        break;
                    }

                    if let GameEvent::GameResult(_) = &event {
                        session.id = 0;
                        break;
                    }
                }


            });
        } else {
            ctx.bot.send_message(message.chat.id, "Only game leader can start the game").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    respond(())
}

fn get_user_id(info: &GameInfo, chat_id: ChatId) -> game::ID {
    info.players.iter()
        .position(|&id| { id == chat_id })
        .map(|id| { id as u8 })
        .unwrap()
}

async fn handle_finish_suggestion(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()>
{
    println!(">handle_finish_suggestion");
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        if let Some(suggestion) = session.suggestion.take() {
            let info = session.info.as_mut().unwrap();
            let mut cli = info.cli.clone();

            let user_id = get_user_id(info, message.chat.id);
            if let Err(e) = cli.suggest_team(user_id, &suggestion.users).await {
                ctx.bot.send_message(message.chat.id, e.to_string()).await?;
                // In case of error, restore the suggestion
                session.suggestion = Some(suggestion);
            } else {
                ctx.bot.send_message(message.chat.id, "Suggestion sent").await?;
            }
        } else {
            ctx.bot.send_message(message.chat.id, "No suggestion in progress").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    println!("<handle_finish_suggestion");
    respond(())
}

async fn handle_team_suggestion(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()> {
    println!(">handle_team_suggestion");
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        let info = session.info.as_ref().unwrap().clone();

        if let Some(suggestions) = session.suggestion.as_mut() {
            let suggest_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
            if let Some(suggest_id) = suggest_cmd.get(1) {
                if let Some(suggest_id) = suggest_id.parse::<u8>().ok() {
                    if let Some(pos) = suggestions.users.iter().position(|&id| { id == suggest_id }) {
                        suggestions.users.remove(pos);
                    } else {
                        suggestions.users.push(suggest_id);
                    }
                    let ctrl_msg = game_msg::suggestion_state(
                        &info, suggestions.crown_id,
                        suggestions.team_size, &suggestions.users);

                    assert_ne!(ctrl_msg.dst, game_msg::Dst::All);
                    let text_msg = control_message_to_string(&ctrl_msg);
                    println!("Suggestion state: {}", text_msg);
                    ctx.bot.edit_message_text(message.chat.id, suggestions.msg_id, text_msg).await?;
                } else {
                    ctx.bot.send_message(message.chat.id, "Invalid suggestion command").await?;
                }
            } else {
                ctx.bot.send_message(message.chat.id, "Invalid suggestion command").await?;
            }
        } else {
            ctx.bot.send_message(message.chat.id, "No suggestion in progress").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    println!("<handle_team_suggestion");
    respond(())
}

async fn handle_team_vote(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()> {
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.clone();
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
                    ctx.bot.send_message(message.chat.id, "Invalid vote command").await?;
                }
            }
        } else {
            ctx.bot.send_message(message.chat.id, "Invalid vote command").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    respond(())
}

async fn handle_mission_result(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()> {
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.clone();
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
                    ctx.bot.send_message(message.chat.id, "Invalid result command").await?;
                }
            }
        } else {
            ctx.bot.send_message(message.chat.id, "Invalid result command").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    respond(())
}

async fn handle_mermaid(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()> {
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.clone();
        let mermaid_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(check_id) = mermaid_cmd.get(1) {
            if let Some(check_id) = check_id.parse::<u8>().ok() {
                cli.send_mermaid_selection(check_id).await.unwrap();
            } else {
                ctx.bot.send_message(message.chat.id, "Invalid mermaid command").await?;
            }
        } else {
            ctx.bot.send_message(message.chat.id, "Invalid mermaid command").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    respond(())
}

async fn handle_mermaid_word(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()> {
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.clone();
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
                    ctx.bot.send_message(message.chat.id, "Invalid mermaid word").await?;
                }
            }
        } else {
            ctx.bot.send_message(message.chat.id, "Invalid mermaid word").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
    }

    respond(())
}

async fn handle_last_chance(ctx: &mut BotCtx, message: &Message) -> ResponseResult<()> {
    if let Some(session) = get_game_session(ctx, message).await {
        let mut session = session.lock().await;
        let info = session.info.as_mut().unwrap();
        let mut cli = info.cli.clone();
        let merlin_cmd = message.text().unwrap().split("_").collect::<Vec<_>>();
        if let Some(merlin_id) = merlin_cmd.get(1) {
            if let Some(merlin_id) = merlin_id.parse::<u8>().ok() {
                cli.send_merlin_check(merlin_id).await.unwrap();
            } else {
                ctx.bot.send_message(message.chat.id, "Invalid last chance command").await?;
            }
        } else {
            ctx.bot.send_message(message.chat.id, "Invalid last chance command").await?;
        }
    } else {
        send_not_in_game(&ctx.bot, message).await?;
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
                handle_start_bot(ctx.deref_mut(), &message, args).await
            }
            "/new_game" => {
                handle_new_game(ctx.deref_mut(), &message).await
            }
            "/start_game" => {
                handle_start_game(ctx.deref_mut(), &message).await
            }
            "/exit" => {
                handle_exit(ctx.deref_mut(), &message).await
            }

            "/suggest_finish" => {
                handle_finish_suggestion(ctx.deref_mut(), &message).await
            }

            cmd if cmd.starts_with("/suggest") => {
                handle_team_suggestion(ctx.deref_mut(), &message).await
            }

            cmd if cmd.starts_with("/team") => {
                handle_team_vote(ctx.deref_mut(), &message).await
            }

            cmd if cmd.starts_with("/mission") => {
                handle_mission_result(ctx.deref_mut(), &message).await
            }

            cmd if cmd.starts_with("/mermaid") => {
                handle_mermaid(ctx.deref_mut(), &message).await
            }

            cmd if cmd.starts_with("/say") => {
                handle_mermaid_word(ctx.deref_mut(), &message).await
            }

            cmd if cmd.starts_with("/merlin") => {
                handle_last_chance(ctx.deref_mut(), &message).await
            }

            _ => {
                bot.send_message(message.chat.id, "Unknown command").await?;
                respond(())
            }
        }
    } else {
        respond(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bot = Bot::from_env();
    let ctx = Arc::new(Mutex::new(BotCtx {
        bot: bot.clone(),
        last_game_id: 0,
        user_games: HashMap::new(),
        game_sessions: HashMap::new(),
        user_names: HashMap::new(),
    }));

    teloxide::repl(bot, move |bot: Bot, message: Message| {
        let ctx = ctx.clone();
        async move { handle_tg_message(bot, message, ctx).await }
    }).await;


    Ok(())
}