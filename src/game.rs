use std::error::Error;
use std::vec::Vec;
use rand::{seq::SliceRandom, RngCore};
use futures::{executor, channel::{oneshot::{self, Sender, Receiver}, mpsc::{self, UnboundedSender, UnboundedReceiver}}, SinkExt};

/*
Start:
  1. A Creator creates game session for N players
  2. Collect responses from players (+names)
  3. Start game (at any time the Creator can stop the game session)
  4. Send roles.
  5. Send introduction test to the Creator
  6. Wait for a feedback about end of intro
  7. Assign the crown
  8. Assign the mermaid

Game:
  1. Check whether game is over
  1. Check if mermaid should be used, give it a move
  1. Person with the crown suggests a team
  1. All people votes for the team for a mission
  1. If the team was rejected, increase rejected counter, move the crown
  1. Otherwise set rejected counter to zero and start the mission
  1. During the mission every person of the team select "success"/"fail"
  1. Check whether mission is succeeded or failed
  1. Move the crown
  1. Repeat
*/

#[derive(PartialEq, Clone)]
enum Role {
    Mordred,
    Morgen,
    Oberon,
    Bad,

    Merlin,
    Percival,
    Good,
}

impl Role {
    pub fn is_good(&self) -> bool {
        match self {
            Role::Merlin |
            Role::Percival |
            Role::Good => true,

            _ => false
        }
    }
}

type ID=u8;

struct Person {
    id: ID,
    role: Role,
}

#[derive(PartialEq, Clone)]
enum TeamVote {
    Approve,
    Reject
}

#[derive(PartialEq, Clone)]
enum MissionVote {
    Success,
    Fail
}

struct Mission {
    team: Vec<Person>,
    votes: Vec<bool>,
}

#[derive(PartialEq, Clone, Debug)]
enum GameResult {
    GoodWins,
    BadWins
}

const MAX_TRY_COUNT: u8 = 5;

pub struct Game {
    team_channels: (Sender<Vec<ID>>, Receiver<Vec<ID>>),
    vote_channels: (UnboundedSender<Vec<TeamVote>>, UnboundedReceiver<Vec<TeamVote>>),
    mission_channels: (Sender<Vec<MissionVote>>, Receiver<Vec<MissionVote>>),

    players: Vec<Role>,

    votes: Vec<Option<TeamVote>>,

    expected_team_size: u8,
    current_team: Vec<ID>, // team for the mission
    mission_votes: Vec<MissionVote>,

    crown_id: ID,
    try_count: u8,

    missions: Vec<MissionVote>
}

fn is_mission_approved(votes: Vec<TeamVote>) -> bool {
    if votes.len() == 0 {
        return false
    }

    let approve_cnt = votes.iter()
        .filter(|x| **x == TeamVote::Approve)
        .count();

    return approve_cnt * 2 > votes.len();
}

fn get_expected_team_size(mission: usize,
                          players: usize) -> Option<u8> {
    if mission > 5 {
        return None
    }

    if players < 5 || players > 10 {
        return None;
    }

    static TEAM_SIZE_TABLE: &'static [[usize; 6]; 5] = &[
        [2, 2, 2, 3, 3, 3],
        [3, 3, 3, 4, 4, 4],
        [2, 4, 3, 4, 4, 4],
        [3, 3, 4, 5, 5, 5],
        [3, 4, 4, 5, 5, 5],
    ];

    return Some(TEAM_SIZE_TABLE[mission][players - 5] as u8);
}

fn calc_mission_result(mission: usize,
                       players: usize,
                       mission_votes: Vec<MissionVote>) -> MissionVote {
    let fails_count = mission_votes.iter()
        .filter(|x| **x == MissionVote::Fail)
        .count();

    let success = if players > 7 && mission == 4 {
        fails_count < 2
    } else {
        fails_count == 0
    };

    if success {
        MissionVote::Success
    } else {
        MissionVote::Fail
    }
}

fn calc_winner(mission_votes: Vec<MissionVote>) -> Option<GameResult> {
    let fails_count = mission_votes.iter()
        .filter(|x| **x == MissionVote::Fail)
        .count();
    let success_count = mission_votes.len() - fails_count;

    if fails_count >= 3 {
        Some(GameResult::BadWins)
    } else if success_count >= 3 {
        Some(GameResult::GoodWins)
    } else {
        None
    }
}

impl Game {
    pub fn setup(number: usize) -> Game {
        let mut rng = rand::thread_rng();

        let mut g = Game {
            team_channels: oneshot::channel::<Vec<ID>>(),
            vote_channels: mpsc::unbounded::<Vec<TeamVote>>(),
            mission_channels: oneshot::channel::<Vec<MissionVote>>(),

            players: match number {
                5 => vec!(Role::Mordred, Role::Morgen,
                        Role::Merlin, Role::Good, Role::Good),
                6 => vec!(Role::Mordred, Role::Morgen,
                        Role::Merlin, Role::Percival, Role::Good, Role::Good),
                7 => vec!(Role::Mordred, Role::Morgen, Role::Oberon,
                        Role::Merlin, Role::Percival, Role::Good, Role::Good),
                _ => panic!("Not supported number of players")
            },

            missions: Vec::new(),
            votes: Vec::new(),
            current_team: Vec::new(),
            mission_votes: Vec::new(),

            try_count: 0,
            expected_team_size: 0,
            crown_id: (rng.next_u32() % number as u32) as ID,
        };

        g.votes.resize(number, Option::None);
        g.players.shuffle(& mut rng);

        g
    }

    pub fn get_players(&self) -> Vec<Role> {
        self.players.clone()
    }

    pub fn get_crown(&self) -> ID {
        self.crown_id
    }

    pub fn get_mermaid(&self) -> ID {
        let num = self.players.len();
        (self.crown_id - 1) % num as ID
    }

    fn next_turn(&mut self) {
        let num = self.players.len();
        self.crown_id = (self.crown_id + 1) % num as ID
    }

    pub fn suggest_team(&self, from: ID, suggested_team: Vec<ID>) -> Result<(), Box<dyn Error>> {
        if from != self.get_crown() {
            return Err("Teammate can only be added by crown holder".into())
        }

        if suggested_team.len() != self.expected_team_size as usize {
            return Err("Team is not full".into())
        }

        let (tx, _) = &self.team_channels;
        tx.send(suggested_team);
        Ok(())
    }

    pub fn add_vote(&self, from: ID, vote: TeamVote) -> Result<(), Box<dyn Error>> {
        self.votes[from as usize] = Some(vote);

        if !self.votes.contains(&Option::None) {
            let votes = self.votes.iter()
                .map(|x| x.unwrap())
                .collect();
            self.vote_channels.0.send(votes);
        }
        Ok(())
    }

    pub fn submit_for_mission(&self, from: ID, vote: MissionVote) -> Result<(), Box<dyn Error>> {
        if !self.current_team.contains(&from) {
            return Err("Vote can only be sent by current team player".into())
        }

        if self.players[from as usize].is_good() && vote == MissionVote::Fail {
            return Err("Good player could vote only with Success".into())
        }

        self.mission_votes.push(vote);

        if self.mission_votes.len() == self.current_team.len() {
            self.mission_channels.0.send(self.mission_votes);
        }
        Ok(())
    }

    pub fn start(&self) -> Result<GameResult, Box<dyn Error>> {
        let fut = async {
            let mut mission_votes = Vec::<MissionVote>::new();
            let team_votes = Vec::<TeamVote>::new();

            let current_mission = self.missions.len() + 1;
            let number_of_players = self.players.len();

            self.expected_team_size = get_expected_team_size(current_mission,
                                                             number_of_players).unwrap();

            while calc_winner(mission_votes) == None {
                while !is_mission_approved(team_votes) && self.try_count < MAX_TRY_COUNT {
                    // Wait for the team
                    self.current_team = self.team_channels.1.await?;

                    println!("Suggested team: {:?}", self.current_team);

                    team_votes = self.vote_channels.1.await?;
                    self.try_count += 1;
                }

                if self.try_count == MAX_TRY_COUNT {
                    return Ok(GameResult::BadWins);
                }

                self.try_count = 0;

                mission_votes = self.mission_channels.1.await?;

                let result = calc_mission_result(current_mission,
                    number_of_players, mission_votes);

                // TODO: Send notification to a separate channel
                self.missions.push(result);
            }

            // TODO: Give a chance to guess Merlin
            Ok(calc_winner(mission_votes).unwrap())
        };

        executor::block_on(fut)
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{MissionVote, calc_winner, GameResult};

    #[test]
    fn check_winner_calculator() {
        let mut mission_votes = Vec::<MissionVote>::new();
        assert_eq!(calc_winner(mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(mission_votes), Some(GameResult::GoodWins));
    }
}