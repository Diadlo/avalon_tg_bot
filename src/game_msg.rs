use std::error::Error;

use teloxide::types::ChatId;

use crate::{game::{GameEvent, TeamVote, self, MissionVote, Team, GameResult}, GameInfo};

#[derive(PartialEq, Debug)]
pub enum Dst {
    All,
    User(ChatId)
}

#[derive(Debug)]
pub struct Notification {
    pub dst: Dst,
    pub message: String,
}

#[derive(Debug)]
pub struct ControlMessage {
    pub dst: Dst,
    pub message: String,
    pub commands: Vec<String>,
}

#[derive(Debug)]
pub enum GameMessage {
    Notification(Notification),
    ControlMessage(ControlMessage),
}

struct SuggestionUser {
    id: u8,
    name: String,
    selected: bool,
}

impl GameMessage {
    fn turn(crown_name: &str, team_size: usize, results: Vec<MissionVote>) -> Self {
        let mission_history = results.iter()
            .map(|vote| {
                if vote == &MissionVote::Success { "🏆" } else { "🗡️" }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let mission_chose = format!("{} chooses a team of {} people", crown_name, team_size),
        Self::Notification(Notification {
            dst: Dst::All,
            message: format!("{}\n{}", mission_history, mission_chose),
        })
    }

    fn turn_ctrl_raw(crown_id: ChatId, team_size: usize, users: &[SuggestionUser]) -> ControlMessage {
        let mut users = users.iter()
            .map(|user| {
                let icon = if user.selected { "☑️ " } else { "" };
                format!("suggest_{} {}{}", user.id, icon, user.name)
            })
            .collect::<Vec<_>>();

        users.push("suggest_finish".to_string());

        ControlMessage {
            dst: Dst::User(crown_id),
            message: format!("You chooses a team of {} people", team_size),
            commands: users,
        }
    }

    fn turn_ctrl(crown_id: ChatId, team_size: usize, users: &[SuggestionUser]) -> Self {
        Self::ControlMessage(Self::turn_ctrl_raw(crown_id, team_size, users))
    }

    fn suggested_team(team_names: &[&str]) -> Self {
        let message = format!("Suggested team: {}", team_names.join(", "));

        Self::Notification(Notification {
            dst: Dst::All,
            message,
        })
    }

    fn team_vote_ctrl() -> Self {
        Self::ControlMessage(ControlMessage {
            dst: Dst::All,
            message: "Vote".to_string(),
            commands: vec!["team_approve".to_string(), "team_reject".to_string()],
        })
    }

    fn team_votes(votes: &[(&str, TeamVote)]) -> Self {
        let message = format!("Votes: \n{}", votes.iter()
            .map(|(name, vote)| {
                format!("{} - {} {}", name, if vote == &TeamVote::Approve { "⚪" } else { "⚫" }, vote)
            })
            .collect::<Vec<_>>()
            .join("\n"));

        Self::Notification(Notification {
            dst: Dst::All,
            message,
        })
    }

    fn team_approved() -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: "Team approved".to_string(),
        })
    }

    fn on_mission_ctrl(chat_id: ChatId) -> Self {
        Self::ControlMessage(ControlMessage {
            dst: Dst::User(chat_id),
            message: "You are on the mission. Select your result".to_string(),
            commands: vec!["mission_success".to_string(), "mission_fail".to_string()],
        })
    }

    fn team_rejected(try_count: u8) -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: format!("Team rejected. Try count: {}/{}", try_count, game::MAX_TRY_COUNT)
        })
    }

    fn mission_result(results: &[MissionVote]) -> Self {
        let message = format!("Mission results: {}", results.iter().map(|result| {
            format!("{} {}", if result == &MissionVote::Success { "🏆" } else { "🗡️" }, result)
        }).collect::<Vec<_>>().join(", "));

        Self::Notification(Notification {
            dst: Dst::All,
            message,
        })
    }

    fn mermaid_turn(mermaid_name: &str) -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: format!("{} is going to use mermaid", mermaid_name),
        })
    }

    fn mermaid_ctrl(users: &[(u8, &str)]) -> Self {
        let users = users.iter()
            .map(|(id, name)| {
                format!("mermaid_{} {}", id, name)
            })
            .collect::<Vec<_>>();

        Self::ControlMessage(ControlMessage {
            dst: Dst::All,
            message: "Use mermaid. Select user to check".to_string(),
            commands: users,
        })
    }

    fn mermaid_result(mermaid_id: ChatId, user: &str, team: Team) -> Self {
        let message = format!("Mermaid says {} is {}", user, team);

        Self::Notification(Notification {
            dst: Dst::User(mermaid_id),
            message,
        })
    }

    fn mermaid_word_ctrl(mermaid_id: ChatId) -> Self {
        Self::ControlMessage(ControlMessage {
            dst: Dst::User(mermaid_id),
            message: "Select what to announce".to_string(),
            commands: vec!["say_good".to_string(), "say_bad".to_string()],
        })
    }

    fn mermaid_word(mermaid_name: &str, user: &str, team: Team) -> Self {
        let message = format!("{} says {} is {}", mermaid_name, user, team);

        Self::Notification(Notification {
            dst: Dst::All,
            message,
        })
    }

    fn intermediate_good_win() -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: "Good team are winning, but bad team has one last chance".to_string()
        })
    }

    fn announce_bad_team(bad_team: &[&str]) -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: format!("Bad team: {}", bad_team.join(", ")),
        })
    }

    fn announce_merlin_guesser(guesser: &str) -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: format!("{} is going to guess Merlin", guesser),
        })
    }

    fn last_chance_ctrl(guesser_id: ChatId, good_team: &[(u8, &str)]) -> Self {
        let good_team = good_team.iter()
            .map(|(id, name)| {
                format!("merlin_{} {}", id, name)
            })
            .collect::<Vec<_>>();

        Self::ControlMessage(ControlMessage {
            dst: Dst::User(guesser_id),
            message: "Enter /merlin <id> to check user".to_string(),
            commands: good_team,
        })
    }

    fn announce_merlin(merlin_name: &str) -> Self {
        Self::Notification(Notification {
            dst: Dst::All,
            message: format!("Merlin is {}", merlin_name),
        })
    }

    fn game_result(result: GameResult) -> Self {
        let message = if result == GameResult::GoodWins {
            "Good team won!"
        } else {
            "Bad team won!"
        };

        Self::Notification(Notification {
            dst: Dst::All,
            message: message.to_string(),
        })
    }
}

fn get_user_chat_id(info: &GameInfo, id: u8) -> ChatId {
    info.players[id as usize]
}

fn get_user_name(info: &GameInfo, id: u8) -> &str {
    let chat_id = get_user_chat_id(info, id);
    info.user_names.get(&chat_id).unwrap()
}

fn get_user_name_by_chat<'a>(info: &'a GameInfo, chat_id: &ChatId) -> &'a str {
    info.user_names.get(chat_id).unwrap()
}

pub async fn build_message_for_event(info: &GameInfo, event: GameEvent) -> Result<Vec<GameMessage>, Box<dyn Error>>
{
    match event {
        GameEvent::Turn(crown_id, team_size) => {
            println!("Turn: crown_id={} team_size={}", crown_id, team_size);
            let crown_chat_id = get_user_chat_id(info, crown_id);
            let crown_name = get_user_name(info, crown_id);
            let player_num = info.players.len() as u8;

            let users = (0..player_num)
                .map(|id| {
                    SuggestionUser {
                        id,
                        name: get_user_name(info, id).to_string(),
                        selected: false,
                    }
                })
                .collect::<Vec<_>>();

            Ok(vec![
                GameMessage::turn(crown_name, team_size, &info.results),
                GameMessage::turn_ctrl(crown_chat_id, team_size, &users)
            ])
        },
        GameEvent::TeamSuggested(team) => {
            let team_names = team.iter().map(|id| {
                get_user_name(info, *id)
            });

            Ok(vec![
                GameMessage::suggested_team(&team_names.collect::<Vec<_>>()),
                GameMessage::team_vote_ctrl(),
            ])
        },
        GameEvent::TeamVote(votes) => {
            let player_votes = info.players.iter()
                .zip(votes)
                .map(|(chat_id, vote)| {
                    let name = get_user_name_by_chat(info, chat_id);
                    (name, vote)
                })
                .collect::<Vec<_>>();

            Ok(vec![GameMessage::team_votes(&player_votes)])
        },
        GameEvent::TeamApproved(team) => {
            let mut messages = vec![GameMessage::team_approved()];

            for player in &team {
                let chat_id = get_user_chat_id(info, *player);
                messages.push(GameMessage::on_mission_ctrl(chat_id));
            }

            Ok(messages)
        },
        GameEvent::TeamRejected(try_count) => {
            Ok(vec![GameMessage::team_rejected(try_count)])
        },
        GameEvent::MissionResult(results) => {
            Ok(vec![GameMessage::mission_result(&results)])
        },
        GameEvent::Mermaid(mermaid_id) => {
            let mermaid_name = get_user_name(info, mermaid_id);
            let player_num = info.players.len() as u8;

            let users = (0..player_num)
                .filter(|id| *id != mermaid_id)
                .map(|id| {
                    let username = get_user_name(info, id);
                    (id as u8, username)
                })
                .collect::<Vec<_>>();

            Ok(vec![
                GameMessage::mermaid_turn(mermaid_name),
                GameMessage::mermaid_ctrl(&users),
            ])
        },
        GameEvent::MermaidResult(mermaid_id, checked_user, team) => {
            let mermaid_chat_id = get_user_chat_id(info, mermaid_id);

            let checked_user_name = get_user_name(info, checked_user);

            Ok(vec![
                GameMessage::mermaid_result(mermaid_chat_id, checked_user_name, team),
                GameMessage::mermaid_word_ctrl(mermaid_chat_id),
            ])
        },
        GameEvent::MermaidSays(checked_user, team) => {
            let checked_user_name = get_user_name(info, checked_user);
            let mermaid_id = info.cli.get_mermaid_id().await;
            let mermaid_user_name = get_user_name(info, mermaid_id);
            Ok(vec![GameMessage::mermaid_word(mermaid_user_name, checked_user_name, team)])
        },
        GameEvent::BadLastChance(bad_team, guesser) => {
            let bad_team_names = bad_team.iter().map(|id| {
                get_user_name(info, *id)
            }).collect::<Vec<_>>();

            let guesser_chat_id = get_user_chat_id(info, guesser);
            let guesser_name = get_user_name(info, guesser);
            let player_num = info.players.len() as u8;

            let good_team = (0..player_num)
                .filter(|id| { !bad_team.contains(&(*id as u8)) })
                .map(|id| { (id, get_user_name(info, id)) })
                .collect::<Vec<_>>();

            Ok(vec![
                GameMessage::intermediate_good_win(),
                GameMessage::announce_bad_team(&bad_team_names),
                GameMessage::announce_merlin_guesser(guesser_name),
                GameMessage::last_chance_ctrl(guesser_chat_id, &good_team),
            ])
        },
        GameEvent::Merlin(merlin_id) => {
            let merlin_name = get_user_name(info, merlin_id);
            Ok(vec![GameMessage::announce_merlin(merlin_name)])
        },
        GameEvent::GameResult(result) => {
            Ok(vec![GameMessage::game_result(result)])
        },
    }
}

pub fn suggestion_state(info: &GameInfo, crown_id: u8, team_size: usize, selected_team: &[u8]) -> ControlMessage {
    let crown_chat_id = get_user_chat_id(info, crown_id);
    let player_num = info.players.len() as u8;

    let users = (0..player_num)
        .map(|id| {
            SuggestionUser {
                id,
                name: get_user_name(info, id).to_string(),
                selected: selected_team.contains(&id),
            }
        })
        .collect::<Vec<_>>();

    GameMessage::turn_ctrl_raw(crown_chat_id, team_size, &users)
}
