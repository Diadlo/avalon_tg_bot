use std::error::Error;
use std::vec::Vec;
use rand::{seq::SliceRandom, RngCore};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

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

#[derive(PartialEq, Clone, Debug)]
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

#[derive(PartialEq, Clone, Debug)]
pub enum GameResult {
    GoodWins,
    BadWins
}

const MAX_TRY_COUNT: u8 = 5;

pub struct Game {
    team_channels:    (mpsc::UnboundedSender<Vec<ID>>,          mpsc::UnboundedReceiver<Vec<ID>>),
    vote_channels:    (mpsc::UnboundedSender<Vec<TeamVote>>,    mpsc::UnboundedReceiver<Vec<TeamVote>>),
    mission_channels: (mpsc::UnboundedSender<Vec<MissionVote>>, mpsc::UnboundedReceiver<Vec<MissionVote>>),

    players: Vec<Role>,

    votes: Vec<Option<TeamVote>>,

    expected_team_size: u8,
    current_team: Vec<ID>, // team for the mission
    mission_votes: Vec<MissionVote>,

    crown_id: ID,
    try_count: u8,

    missions: Vec<MissionVote>
}

fn is_mission_approved(votes: &Vec<TeamVote>) -> bool {
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
                       mission_votes: &Vec<MissionVote>) -> MissionVote {
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

fn calc_winner(mission_votes: &Vec<MissionVote>) -> Option<GameResult> {
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
            team_channels: mpsc::unbounded_channel(),
            vote_channels: mpsc::unbounded_channel(),
            mission_channels: mpsc::unbounded_channel(),

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

    pub fn add_vote(&mut self, from: ID, vote: TeamVote) -> Result<(), Box<dyn Error>> {
        self.votes[from as usize] = Some(vote);

        if !self.votes.contains(&Option::None) {
            let votes = self.votes.iter()
                .map(|x| x.clone().unwrap())
                .collect();
            self.vote_channels.0.send(votes);
        }
        Ok(())
    }

    pub fn submit_for_mission(&mut self, from: ID, vote: MissionVote) -> Result<(), Box<dyn Error>> {
        if !self.current_team.contains(&from) {
            return Err("Vote can only be sent by current team player".into())
        }

        if self.players[from as usize].is_good() && vote == MissionVote::Fail {
            return Err("Good player could vote only with Success".into())
        }

        self.mission_votes.push(vote);

        if self.mission_votes.len() == self.current_team.len() {
            self.mission_channels.0.send(self.mission_votes.clone());
        }
        Ok(())
    }

    pub fn get_missions(&self) -> Vec<MissionVote> {
        return self.missions.clone()
    }

    pub async fn start(&mut self) -> Result<GameResult, Box<dyn Error>> {
        let mut mission_votes = Vec::<MissionVote>::new();
        let mut team_votes = Vec::<TeamVote>::new();

        let current_mission = self.missions.len() + 1;
        let number_of_players = self.players.len();

        self.expected_team_size = get_expected_team_size(current_mission,
                                                            number_of_players).unwrap();

        while calc_winner(&mission_votes) == None {
            while !is_mission_approved(&team_votes) && self.try_count < MAX_TRY_COUNT {
                // Wait for the team
                self.current_team = self.team_channels.1.recv().await.unwrap();

                println!("Suggested team: {:?}", self.current_team);

                team_votes = self.vote_channels.1.recv().await.unwrap();
                self.try_count += 1;
            }

            if self.try_count == MAX_TRY_COUNT {
                return Ok(GameResult::BadWins);
            }

            self.try_count = 0;

            mission_votes = self.mission_channels.1.recv().await.unwrap();

            let result = calc_mission_result(current_mission,
                number_of_players, &mission_votes);

            // TODO: Send notification to a separate channel
            self.missions.push(result);
        }

        // TODO: Give a chance to guess Merlin
        Ok(calc_winner(&mission_votes).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use crate::game::{Game, TeamVote, MissionVote, calc_winner, GameResult};

    #[test]
    fn calc_clear_good_winner() {
        let mut mission_votes = Vec::<MissionVote>::new();
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), Some(GameResult::GoodWins));
    }

    #[test]
    fn calc_clear_bad_winner() {
        let mut mission_votes = Vec::<MissionVote>::new();
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), Some(GameResult::BadWins));
    }

    #[test]
    fn calc_winner_mixed_good() {
        let mut mission_votes = Vec::<MissionVote>::new();
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), Some(GameResult::GoodWins));
    }

    #[test]
    fn calc_winner_mixed_bad() {
        let mut mission_votes = Vec::<MissionVote>::new();
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Success);
        assert_eq!(calc_winner(&mission_votes), None);

        mission_votes.push(MissionVote::Fail);
        assert_eq!(calc_winner(&mission_votes), Some(GameResult::BadWins));
    }

    fn all_votes_approve(g: &mut Game) {
        for i in 0..7 {
            g.add_vote(i, TeamVote::Approve);
        }
    }

    #[tokio::test]
    async fn test_game() {
        let mut g = Game::setup(7);
        let game_fut = async {
            let result = g.start().await.unwrap();
            assert_eq!(result, GameResult::GoodWins);
        };

        let test_fut = async {
            println!("Players: {:?}", g.get_players());
            g.suggest_team(0, vec![0, 1]);

            all_votes_approve(&mut g);

            g.submit_for_mission(0, MissionVote::Success);
            g.submit_for_mission(1, MissionVote::Success);
            g.submit_for_mission(2, MissionVote::Success);
        };

        tokio::join!(game_fut, test_fut);
    }
}