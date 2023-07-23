mod game;

use std::{sync::Arc, ops::{Deref, DerefMut}, collections::HashMap};

use teloxide::prelude::*;
use tokio::sync::Mutex;

const BOT_TG_ADDR: &str = "the_resistance_avalon_bot";

struct BotCtx {
    last_game_id: u32,
    user_games: HashMap<ChatId, u32>,
    games: HashMap<u32, GameInfo>
}

#[derive(Clone)]
struct GameInfo {
    game_id: u32,
    leader: ChatId,
    players: Vec<ChatId>,
}

async fn handle_start_bot<'a, I>(bot: &Bot, message: &Message, ctx: &mut BotCtx, mut cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
{
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        // If there is entry in user_games, game entry must exist
        let _ = ctx.games.get(game_id).unwrap();
        bot.send_message(message.chat.id, "You are already in the game").await?;
        bot.send_message(message.chat.id, "If you want to leave it, use /exit command, than join the link again").await?;
    } else {
        if let Some(param) = cmd.next() {
            if let Ok(game_id) = param.parse::<u32>() {
                if let Some(info) = ctx.games.get_mut(&game_id) {
                    bot.send_message(message.chat.id, "You are joined the game").await?;
                    bot.send_message(message.chat.id, "Wait for the game to start").await?;
                    let name = if let Some(user) = &message.from() {
                        user.first_name.clone()
                    } else {
                        message.chat.id.to_string()
                    };

                    bot.send_message(info.leader, format!("{} joined the game", name)).await?;
                    ctx.user_games.insert(message.chat.id, game_id);
                    info.players.push(message.chat.id);
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
        if let Some(info) = ctx.games.get(game_id) {
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
    let info = GameInfo {
        game_id: ctx.last_game_id + 1,
        leader: message.chat.id,
        players: vec![message.chat.id],
    };

    ctx.games.insert(info.game_id, info.clone());
    ctx.user_games.insert(message.chat.id, info.game_id);
    ctx.last_game_id += 1;

    let id = message.chat.id;
    bot.send_message(id, "Starting a new game...").await?;
    bot.send_message(id, "Send the following invite link to your team").await?;
    let url = format!("https://t.me/the_resistance_avalon_bot?start={}", info.game_id);
    bot.send_message(id, url).await?;

    respond(())
}

async fn handle_start_game<'a, I>(bot: &Bot, message: &Message, ctx: &mut BotCtx, cmd: I) -> ResponseResult<()>
    where I: Iterator<Item = &'a str>
{
    if let Some(game_id) = ctx.user_games.get(&message.chat.id) {
        // If there is entry in user_games, game entry must exist
        let info = ctx.games.get(game_id).unwrap();
        if info.leader == message.chat.id {
            bot.send_message(message.chat.id, "Starting the game...").await?;
            bot.send_message(message.chat.id, "Game started!").await?;
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
        games: HashMap::new()
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
