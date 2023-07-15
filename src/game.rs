use std::{error::Error, sync::Arc};
use std::vec::Vec;
use rand::Rng;
use rand::seq::SliceRandom;
use tokio::sync::{mpsc, Mutex};

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
pub enum Team {
    Good,
    Bad
}

#[derive(PartialEq, Clone, Debug)]
pub enum Role {
    Mordred,
    Morgen,
    Oberon,
    Assassin,
    Bad,

    Merlin,
    Percival,
    Good,
    // The same as Good, but added to simplify code because now team contains only unique roles
    Good2,
}

impl Role {
    pub fn is_good(&self) -> bool {
        match self {
            Role::Merlin |
            Role::Percival |
            Role::Good |
            Role::Good2 => true,

            Role::Mordred |
            Role::Morgen |
            Role::Assassin |
            Role::Oberon |
            Role::Bad => false,
        }
    }
}

type ID=u8;

#[derive(PartialEq, Clone, Debug)]
pub enum TeamVote {
    Approve,
    Reject
}

#[derive(PartialEq, Clone, Debug)]
pub enum MissionVote {
    Success,
    Fail
}

#[derive(PartialEq, Clone, Debug)]
pub enum GameResult {
    GoodWins,
    BadWins
}

const MAX_TRY_COUNT: u8 = 5;

pub struct GameInfo {
    players: Vec<Role>,

    expected_team_size: usize,
    current_team: Vec<ID>, // team for the mission

    mermaid_id: ID,
    crown_id: ID,

    missions: Vec<MissionVote>
}

#[derive(PartialEq, Clone, Debug)]
pub enum GameEvent {
    Turn(ID, usize), // Crown ID, team size for the mission
    TeamSuggested(Vec<ID>),
    TeamVote(Vec<TeamVote>),
    TeamApproved,
    TeamRejected(u8), // Try count
    MissionResult(Vec<MissionVote>),
    Mermaid(ID), // Mermaid ID
    MermaidResult(Team), // Role of the checked player
    MermaidSays(Team), // Mermaid says who is player
    Merlin(ID), // Actual merlin ID
    GameResult(GameResult),
}

pub struct GameClient {
    rx_event:  mpsc::UnboundedReceiver<GameEvent>,

    // Mermaid owner selected player
    tx_mermaid_selection: mpsc::UnboundedSender<ID>,
    // Mermaid says who is player
    tx_mermaid_word: mpsc::UnboundedSender<Team>,

    tx_team:    mpsc::UnboundedSender<Vec<ID>>,
    tx_vote:    mpsc::UnboundedSender<Vec<TeamVote>>,
    tx_mission: mpsc::UnboundedSender<Vec<MissionVote>>,
    tx_merlin:  mpsc::UnboundedSender<ID>,

    votes: Vec<Option<TeamVote>>,
    mission_votes: Vec<MissionVote>,

    info: Arc<Mutex<GameInfo>>,
}

pub struct Game {
    tx_event:  mpsc::UnboundedSender<GameEvent>,

    // Mermaid owner selected player
    rx_mermaid_selection: mpsc::UnboundedReceiver<ID>,
    // Mermaid says who is player
    rx_mermaid_word: mpsc::UnboundedReceiver<Team>,
    // Team was suggested
    rx_team:    mpsc::UnboundedReceiver<Vec<ID>>,
    // Players voted for the suggested team
    rx_vote:    mpsc::UnboundedReceiver<Vec<TeamVote>>,
    // Players voted for the mission
    rx_mission: mpsc::UnboundedReceiver<Vec<MissionVote>>,
    // Bad team tries to guess Merlin
    rx_merlin:  mpsc::UnboundedReceiver<ID>,

    info: Arc<Mutex<GameInfo>>,
}

impl GameClient {
    pub async fn suggest_team(&mut self, from: ID, suggested_team: &Vec<ID>) -> Result<(), Box<dyn Error>> {
        {
            let info = self.info.lock().await;
            if from != info.crown_id {
                return Err("Teammate can only be added by crown holder".into())
            }

            if suggested_team.len() != info.expected_team_size as usize {
                return Err("Team is not full".into())
            }
        }

        self.tx_team.send(suggested_team.clone())?;
        Ok(())
    }

    pub async fn add_team_vote(&mut self, from: ID, vote: TeamVote) -> Result<(), Box<dyn Error>> {
        self.votes[from as usize] = Some(vote);

        if !self.votes.contains(&Option::None) {
            let votes = self.votes.iter()
                .map(|x| x.clone().unwrap())
                .collect();
            println!("send_team_votes");
            self.tx_vote.send(votes)?;

            for i in 0..self.votes.len() {
                self.votes[i] = Option::None;
            }
        }
        Ok(())
    }

    pub async fn submit_for_mission(&mut self, from: ID, vote: MissionVote) -> Result<(), Box<dyn Error>> {
        let enough_votes = {
            let info = self.info.lock().await;

            if !info.current_team.contains(&from) {
                return Err("Vote can only be sent by current team player".into())
            }

            if info.players[from as usize].is_good() && vote == MissionVote::Fail {
                return Err("Good player could vote only with Success".into())
            }

            self.mission_votes.push(vote.clone());

            info.expected_team_size == self.mission_votes.len()
        };

        if enough_votes {
            self.tx_mission.send(self.mission_votes.clone())?;
            self.mission_votes.clear();
        }

        Ok(())
    }

    async fn recv_event(&mut self) -> Result<GameEvent, Box<dyn Error>> {
        let event = self.rx_event.recv().await
            .ok_or("Channel closed")?;
        Ok(event)
    }

    async fn send_mermaid_selection(&mut self, id: ID) -> Result<(), Box<dyn Error>> {
        self.tx_mermaid_selection.send(id)?;
        Ok(())
    }

    async fn send_mermaid_word(&mut self, word: Team) -> Result<(), Box<dyn Error>> {
        self.tx_mermaid_word.send(word)?;
        Ok(())
    }

    async fn send_merlin_check(&mut self, id: ID) -> Result<(), Box<dyn Error>> {
        self.tx_merlin.send(id)?;
        Ok(())
    }
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
                          players: usize) -> Option<usize> {
    let mission = mission - 1;
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

    return Some(TEAM_SIZE_TABLE[mission][players - 5]);
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

fn calc_mermaid_id(crown_id: ID, players: usize) -> ID {
    assert!(crown_id < players as ID);
    let prev_id = crown_id as i32 - 1;
    let mermaid_id = prev_id.rem_euclid(players as i32) as ID;
    mermaid_id
}

fn default_team(players: usize) -> Vec<Role> {
    match players {
        5 => vec!(
            Role::Merlin, Role::Good, Role::Good2,
            Role::Mordred, Role::Morgen,
        ),
        6 => vec!(
            Role::Merlin, Role::Percival, Role::Good, Role::Good2,
            Role::Mordred, Role::Morgen,
        ),
        7 => vec!(
            Role::Merlin, Role::Percival, Role::Good, Role::Good2,
            Role::Mordred, Role::Morgen, Role::Oberon,
        ),
        _ => panic!("Not supported number of players")
    }
}

fn find_role(players: &[Role], search_for: Role) -> ID {
    for (id, role) in players.iter().enumerate() {
        if *role == search_for {
            return id as ID
        }
    }
    panic!("Merlin not found");
}

impl Game {
    pub fn setup(number: usize) -> (Game, GameClient) {
        let (tx_mermaid_selection, rx_mermaid_selection) = mpsc::unbounded_channel();
        let (tx_mermaid_word, rx_mermaid_word) = mpsc::unbounded_channel();
        let (tx_team, rx_team) = mpsc::unbounded_channel();
        let (tx_vote, rx_vote) = mpsc::unbounded_channel();
        let (tx_mission, rx_mission) = mpsc::unbounded_channel();
        let (tx_event, rx_event) = mpsc::unbounded_channel();
        let (tx_merlin, rx_merlin) = mpsc::unbounded_channel();

        let mut rng = rand::thread_rng();
        let crown_id = rng.gen_range(0..number) as ID;

        let mut raw_info = GameInfo {
            players: default_team(number),

            missions: Vec::new(),
            current_team: Vec::new(),

            expected_team_size: 0,
            crown_id,
            // Actually this should be crown_id - 1, but in main loop we do "next_turn"
            // in the beginning, so it will be incremented
            mermaid_id: crown_id,
        };

        raw_info.players.shuffle(&mut rng);

        let info = Arc::new(Mutex::new(raw_info));

        let g = Game {
            tx_event,

            rx_mermaid_selection,
            rx_mermaid_word,
            rx_team,
            rx_vote,
            rx_mission,
            rx_merlin,

            info: info.clone(),
        };

        let mut channels = GameClient {
            rx_event,

            tx_mermaid_selection,
            tx_mermaid_word,
            tx_team,
            tx_vote,
            tx_mission,
            tx_merlin,

            mission_votes: Vec::new(),
            votes: Vec::new(),

            info: info.clone(),
        };

        channels.votes.resize(number, Option::None);

        (g, channels)
    }

    async fn get_mermaid_check(&mut self) -> Result<ID, Box<dyn Error>> {
        {
            let info = self.info.lock().await;
            self.tx_event.send(GameEvent::Mermaid(info.mermaid_id))?;
        }
        let selection = self.rx_mermaid_selection.recv().await
            .ok_or("Channel closed")?;
        Ok(selection)
    }

    async fn get_mermaid_word(&mut self) -> Result<Team, Box<dyn Error>> {
        let word = self.rx_mermaid_word.recv().await
            .ok_or("Channel closed")?;
        Ok(word)
    }

    async fn next_turn(&mut self) -> Result<(), Box<dyn Error>> {
        {
            let mut info = self.info.lock().await;
            let num = info.players.len();
            info.crown_id = (info.crown_id + 1) % num as ID;
        }
        self.update_expected_team_size().await?;
        self.send_turn_event().await?;
        Ok(())
    }

    async fn get_suggested_team(&mut self) -> Vec<ID> {
        self.rx_team.recv().await.unwrap()
    }

    async fn get_team_votes(&mut self) -> Vec<TeamVote> {
        println!("get_team_votes");
        self.rx_vote.recv().await.unwrap()
    }

    async fn get_merlin_check(&mut self) -> Result<ID, Box<dyn Error>> {
        let id = self.rx_merlin.recv().await.ok_or("Channel closed")?;
        Ok(id)
    }

    async fn get_current_mission(&self) -> usize {
        let info = self.info.lock().await;
        info.missions.len() + 1
    }

    async fn get_number_of_players(&self) -> usize {
        let info = self.info.lock().await;
        info.players.len()
    }

    async fn update_expected_team_size(&mut self) -> Result<(), Box<dyn Error>> {
        let mut info = self.info.lock().await;
        info.expected_team_size = get_expected_team_size(info.missions.len()+ 1,
                                                         info.players.len())
                                  .ok_or("Invalid number of players")?;
        Ok(())
    }

    async fn set_current_team(&mut self, team: &Vec<ID>) {
        let mut info = self.info.lock().await;
        info.current_team = team.clone();
        self.tx_event.send(GameEvent::TeamSuggested(team.clone())).unwrap();
    }

    async fn add_mission_result(&mut self, result: MissionVote) {
        let mut info = self.info.lock().await;
        info.missions.push(result);
    }

    fn notify_mission_result(&mut self, mission_votes: &Vec<MissionVote>) -> Result<(), Box<dyn Error>> {
        let mut mission_votes = mission_votes.clone();
        let mut rng = rand::thread_rng();
        mission_votes.shuffle(&mut rng);
        self.tx_event.send(GameEvent::MissionResult(mission_votes))?;
        Ok(())
    }

    async fn calc_winner(&self) -> Option<GameResult> {
        let info = self.info.lock().await;
        calc_winner(&info.missions)
    }

    async fn get_player_team(&self, id: ID) -> Team {
        let info = self.info.lock().await;
        let role = info.players[id as usize].clone();
        if role.is_good() {
            Team::Good
        } else {
            Team::Bad
        }
    }

    async fn get_merlin(&self) -> ID {
        let info = self.info.lock().await;
        find_role(&info.players, Role::Merlin)
    }

    async fn send_mermaid_result(&mut self, team: Team) -> Result<(), Box<dyn Error>> {
        self.tx_event.send(GameEvent::MermaidResult(team))?;
        Ok(())
    }

    async fn send_mermaid_word(&mut self, word: Team) -> Result<(), Box<dyn Error>> {
        self.tx_event.send(GameEvent::MermaidSays(word))?;
        Ok(())
    }

    async fn send_turn_event(&mut self) -> Result<(), Box<dyn Error>> {
        let info = self.info.lock().await;
        self.tx_event.send(GameEvent::Turn(info.crown_id, info.expected_team_size))?;
        Ok(())
    }

    async fn send_team_votes(&mut self, votes: &Vec<TeamVote>) -> Result<(), Box<dyn Error>> {
        println!("Sending team votes: {:?}", votes);
        self.tx_event.send(GameEvent::TeamVote(votes.clone()))?;
        Ok(())
    }

    async fn send_team_vote_result(&mut self, result: GameEvent) -> Result<(), Box<dyn Error>> {
        self.tx_event.send(result)?;
        Ok(())
    }

    async fn send_actual_merlin(&mut self, id: ID) -> Result<(), Box<dyn Error>> {
        self.tx_event.send(GameEvent::Merlin(id))?;
        Ok(())
    }

    async fn send_game_result(&mut self, result: GameResult) -> Result<(), Box<dyn Error>> {
        self.tx_event.send(GameEvent::GameResult(result))?;
        Ok(())
    }

    async fn move_mermaid(&mut self, mermaid_check: ID) -> Result<(), Box<dyn Error>> {
        let mut info = self.info.lock().await;
        println!("Moving mermaid from {} to {}", info.mermaid_id, mermaid_check);
        info.mermaid_id = mermaid_check;
        Ok(())
    }

    pub async fn start(&mut self) -> Result<(), Box<dyn Error>> {
        let current_mission = self.get_current_mission().await;
        let number_of_players = self.get_number_of_players().await;

        while self.calc_winner().await == None {
            let mut try_count = 1;

            loop {
                println!("New turn");
                self.next_turn().await?;

                let team = self.get_suggested_team().await;
                self.set_current_team(&team).await;

                println!("Suggested team: {:?}", team);

                let team_votes = self.get_team_votes().await;
                self.send_team_votes(&team_votes).await?;

                println!("Votes for the team: {:?}", team_votes);

                if is_mission_approved(&team_votes) {
                    println!("Mission approved");
                    self.send_team_vote_result(GameEvent::TeamApproved).await?;
                    break;
                }

                try_count += 1;
                self.send_team_vote_result(GameEvent::TeamRejected(try_count)).await?;
                println!("Mission rejected. Try count: {}", try_count);

                if try_count >= MAX_TRY_COUNT {
                    break;
                }
            }

            if try_count == MAX_TRY_COUNT {
                println!("Too many tries. Bad wins");
                self.send_game_result(GameResult::BadWins).await?;
                return Ok(());
            }

            let mission_votes = self.rx_mission.recv().await.unwrap();
            println!("Mission votes: {:?}", mission_votes);

            let result = calc_mission_result(current_mission,
                number_of_players, &mission_votes);
            println!("Mission result: {:?}", result);

            let mission_idx = self.get_current_mission().await;

            self.add_mission_result(result).await;

            self.notify_mission_result(&mission_votes)?;

            println!("Mission idx: {}", mission_idx);
            if mission_idx > 1 && mission_idx < 5 {
                println!("Waiting for mermaid selection");
                let mermaid_check = self.get_mermaid_check().await?;
                let mermaid_result = self.get_player_team(mermaid_check).await;
                println!("Mermaid sees that {} is {:?}", mermaid_check, mermaid_result);
                self.send_mermaid_result(mermaid_result).await?;
                let mermaid_word = self.get_mermaid_word().await?;
                println!("Mermaid says that player is {:?}", mermaid_word);
                self.send_mermaid_word(mermaid_word).await?;
                self.move_mermaid(mermaid_check).await?;
            }
        }

        let winner = self.calc_winner().await.unwrap();
        if winner == GameResult::BadWins {
            self.send_game_result(winner.clone()).await?;
            return Ok(());
        }

        // If good wins, bad have a chance to win by guessing Merlin
        let merlin_check = self.get_merlin_check().await?;
        let merlin = self.get_merlin().await;

        self.send_actual_merlin(merlin).await?;

        if merlin_check == merlin {
            self.send_game_result(GameResult::BadWins).await?;
            return Ok(());
        }

        self.send_game_result(GameResult::GoodWins).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    fn calc_winner_test(votes: Vec<u32>, expected: Option<GameResult>) {
        let votes: Vec<MissionVote> = votes.into_iter()
            .map(|x| if x != 0 { MissionVote::Success } else { MissionVote::Fail })
            .collect();
        assert_eq!(calc_winner(&votes), expected);
    }

    #[test]
    fn test_winner_calc_no_winner() {
        calc_winner_test(vec![], None);
        calc_winner_test(vec![0], None);
        calc_winner_test(vec![1], None);
        calc_winner_test(vec![0, 0], None);
        calc_winner_test(vec![0, 1], None);
        calc_winner_test(vec![1, 0], None);
        calc_winner_test(vec![1, 1], None);
    }

    #[test]
    fn test_winner_calc_good_wins() {
        calc_winner_test(vec![1, 1, 1], Some(GameResult::GoodWins));
        calc_winner_test(vec![0, 1, 1, 1], Some(GameResult::GoodWins));
        calc_winner_test(vec![0, 1, 0, 1, 1], Some(GameResult::GoodWins));
        calc_winner_test(vec![0, 0, 1, 1, 1], Some(GameResult::GoodWins));
    }

    #[test]
    fn test_winner_calc_bad_wins() {
        calc_winner_test(vec![0, 0, 0], Some(GameResult::BadWins));
        calc_winner_test(vec![1, 0, 0, 0], Some(GameResult::BadWins));
        calc_winner_test(vec![1, 0, 1, 0, 0], Some(GameResult::BadWins));
        calc_winner_test(vec![0, 1, 0, 1, 0], Some(GameResult::BadWins));
    }

    #[test]
    fn test_mermaid_id_overflow() {
        assert_eq!(calc_mermaid_id(2, 3), 1);
        assert_eq!(calc_mermaid_id(1, 3), 0);
        assert_eq!(calc_mermaid_id(0, 3), 2);
    }

    #[test]
    fn test_team_size_for_7_players() {
        assert_eq!(get_expected_team_size(1, 7), Some(2));
        assert_eq!(get_expected_team_size(2, 7), Some(3));
        assert_eq!(get_expected_team_size(3, 7), Some(3));
        assert_eq!(get_expected_team_size(4, 7), Some(4));
        assert_eq!(get_expected_team_size(5, 7), Some(4));
    }

    async fn test_send_team_votes(cli: &mut GameClient, votes: &Vec<TeamVote>) -> Result<(), Box<dyn Error>> {
        for (i, vote) in votes.iter().enumerate() {
            cli.add_team_vote(i as ID, vote.clone()).await?;
        }
        Ok(())
    }

    async fn recv_event(cli: &mut GameClient) -> GameEvent {
        cli.recv_event().await.unwrap()
    }

    fn mission_result_are_equal(a: &Vec<MissionVote>, b: &Vec<MissionVote>) -> bool {
        assert!(a.len() == b.len());
        let a_success_cnt = a.iter().filter(|x| **x == MissionVote::Success).count();
        let b_success_cnt = b.iter().filter(|x| **x == MissionVote::Success).count();
        return a_success_cnt == b_success_cnt;
    }

    #[derive(Clone, Debug)]
    struct MermaidCheck {
        holder: Role,
        selection: Role,
        word: Team,
    }

    struct GameTurn {
        // To make tests easier we will specify roles instead of IDs
        suggestion: Vec<Role>,
        team_votes: Vec<TeamVote>,
        try_count: u8,
        mission_votes: Vec<MissionVote>,
        mermaid_check: Option<MermaidCheck>,
    }

    struct ExpectedGame {
        num: usize,
        players: Vec<Role>,
        start_crown_id: ID,
        turns: Vec<GameTurn>,
        merlin_check: Option<ID>,
        expected_game_result: GameResult,
    }

    fn build_suggested_team(players: &Vec<Role>, roles: &Vec<Role>) -> Vec<ID> {
        let mut team = Vec::new();

        for role in roles {
            let mut id = find_role(&players, role.clone());
            while team.contains(&id) {
                // Find another player with the same role
                // Pass slice of players starting after the previous id of the same role
                id = find_role(&players[id as usize + 1..], role.clone());
            }

            team.push(id);
        }

        team
    }

    async fn cli_find_role(cli: &GameClient, role: Role) -> ID {
        let info = cli.info.lock().await;
        find_role(&info.players, role)
    }

    async fn run_test_game(expected: ExpectedGame) {
        let (mut g, mut cli) = Game::setup(expected.num);

        // During real game players and crown are assigned randomly.
        // But for testing purposes we will assign them manually.
        g.info.lock().await.players = expected.players.clone();
        g.info.lock().await.crown_id = calc_mermaid_id(expected.start_crown_id, expected.num);
        g.info.lock().await.mermaid_id = calc_mermaid_id(expected.start_crown_id, expected.num);

        let game_fut = async {
            g.start().await.unwrap();
            println!("End of game future");
        };

        let test_fut = async {
            for turn in 0..expected.turns.len() {
                let exp_turn = &expected.turns[turn];
                println!("[TEST] turn {}", turn);
                let (crown_id, _) = match recv_event(&mut cli).await {
                    GameEvent::Turn(id, size) => {
                        assert_eq!(size, exp_turn.suggestion.len());
                        (id, size)
                    }
                    event => panic!("Unexpected event: {:?}", event)
                };

                let suggested_team = build_suggested_team(&expected.players, &exp_turn.suggestion);
                println!("[TEST] suggested team: {:?}", suggested_team);
                cli.suggest_team(crown_id, &suggested_team).await.unwrap();

                match recv_event(&mut cli).await {
                    GameEvent::TeamSuggested(suggested) =>
                        assert_eq!(&suggested, &suggested_team),
                    event => panic!("Unexpected event: {:?}", event)
                };

                let expected_votes = &exp_turn.team_votes;
                println!("[TEST] sending team votes: {:?}", expected_votes);
                test_send_team_votes(&mut cli, expected_votes).await.unwrap();

                match recv_event(&mut cli).await {
                    GameEvent::TeamVote(votes) =>
                        assert_eq!(expected_votes, &votes),
                    event => panic!("Unexpected event: {:?}", event)
                };

                match recv_event(&mut cli).await {
                    GameEvent::TeamApproved =>
                        assert!(is_mission_approved(&expected_votes)),
                    GameEvent::TeamRejected(try_cnt) => {
                        assert!(!is_mission_approved(&expected_votes));
                        assert_eq!(try_cnt, exp_turn.try_count);
                        if try_cnt == MAX_TRY_COUNT {
                            break;
                        } else {
                            continue;
                        }
                    }
                    event => panic!("Unexpected event: {:?}", event)
                };

                let mission_votes = &exp_turn.mission_votes;
                println!("[TEST] mission votes: {:?}", mission_votes);
                for (id, vote) in suggested_team.iter().zip(mission_votes.iter()) {
                    cli.submit_for_mission(*id, vote.clone()).await.unwrap();
                }

                match recv_event(&mut cli).await {
                    GameEvent::MissionResult(actual) => {
                        let expected = exp_turn.mission_votes.clone();
                        assert!(mission_result_are_equal(&actual, &expected));
                    }
                    event => panic!("Unexpected event: {:?}", event)
                };

                if let Some(mermaid) = &exp_turn.mermaid_check {
                    println!("[TEST] mermaid: {:?}", mermaid);
                    match recv_event(&mut cli).await {
                        GameEvent::Mermaid(mermaid_id) => {
                            let holder_id = cli_find_role(&mut cli, mermaid.holder.clone()).await;
                            assert_eq!(mermaid_id, holder_id);
                        }
                        event => panic!("Unexpected event: {:?}", event)
                    };

                    let selection_id = cli_find_role(&mut cli, mermaid.selection.clone()).await;
                    cli.send_mermaid_selection(selection_id).await.unwrap();

                    match recv_event(&mut cli).await {
                        GameEvent::MermaidResult(result) => {
                            let expected = if mermaid.selection.is_good() {
                                Team::Good
                            } else {
                                Team::Bad
                            };
                            assert_eq!(result, expected);
                        },
                        event => panic!("Unexpected event: {:?}", event)
                    };

                    cli.send_mermaid_word(mermaid.word.clone()).await.unwrap();

                    match recv_event(&mut cli).await {
                        GameEvent::MermaidSays(word) => {
                            assert_eq!(word, mermaid.word);
                        },
                        event => panic!("Unexpected event: {:?}", event)
                    };
                }
            }

            if let Some(merlin_check) = expected.merlin_check {
                cli.send_merlin_check(merlin_check).await.unwrap();
                match recv_event(&mut cli).await {
                    GameEvent::Merlin(id) => {
                        assert_eq!(id, find_role(&expected.players, Role::Merlin));
                    }
                    event => panic!("Unexpected event: {:?}", event)
                };
            }

            match recv_event(&mut cli).await {
                GameEvent::GameResult(result) => {
                    assert_eq!(result, expected.expected_game_result);
                }
                event => panic!("Unexpected event: {:?}", event)
            };

            // There should be end of the game with GoodWins result
            println!("End of test future");
        };

        tokio::join!(game_fut, test_fut);
    }


    #[tokio::test]
    async fn test_clear_good_game_merlin_is_not_guessed() {
        let expected = ExpectedGame {
            num: 7,
            players: default_team(7),
            start_crown_id: 0,
            turns: vec![
                GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success],
                    mermaid_check: None,
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good, Role::Good2],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Success],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Oberon,
                        selection: Role::Good,
                        word: Team::Good,
                    }),
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good, Role::Good2],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Success],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Good,
                        selection: Role::Good2,
                        word: Team::Good,
                    }),
                }
            ],
            merlin_check: Some(1),
            expected_game_result: GameResult::GoodWins,
        };

        run_test_game(expected).await;
    }

    #[tokio::test]
    async fn test_clear_good_game_but_merlin_is_guessed() {
        let expected = ExpectedGame {
            num: 7,
            players: default_team(7),
            start_crown_id: 0,
            turns: vec![
                GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success],
                    mermaid_check: None,
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good, Role::Good2],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Success],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Oberon,
                        selection: Role::Good,
                        word: Team::Good,
                    }),
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good, Role::Good2],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Success],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Good,
                        selection: Role::Merlin,
                        word: Team::Good,
                    }),
                }
            ],
            merlin_check: Some(0), // 0 is Merlin
            expected_game_result: GameResult::BadWins, // Merlin is guessed
        };

        run_test_game(expected).await;
    }

    #[tokio::test]
    async fn test_bad_wins_due_to_many_rejects() {
        let expected = ExpectedGame {
            num: 7,
            players: default_team(7),
            start_crown_id: 0,
            turns: vec![
                GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good],
                    team_votes: vec![TeamVote::Reject; 7],
                    try_count: 2,
                    mission_votes: vec![],
                    mermaid_check: None,
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good],
                    team_votes: vec![TeamVote::Reject; 7],
                    try_count: 3,
                    mission_votes: vec![],
                    mermaid_check: None,
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good],
                    team_votes: vec![TeamVote::Reject; 7],
                    try_count: 4,
                    mission_votes: vec![],
                    mermaid_check: None,
                }, GameTurn {
                    suggestion: vec![Role::Merlin, Role::Good],
                    team_votes: vec![TeamVote::Reject; 7],
                    try_count: 5,
                    mission_votes: vec![],
                    mermaid_check: None,
                }
            ],
            merlin_check: None, // No merlin check
            expected_game_result: GameResult::BadWins,
        };

        run_test_game(expected).await;
    }

    #[tokio::test]
    async fn test_game_with_fail_on_fourth_mission_and_one_reject() {
        let expected = ExpectedGame {
            num: 7,
            players: default_team(7),
            start_crown_id: 0,
            turns: vec![
                // Success
                GameTurn {
                    suggestion: vec![Role::Good, Role::Good2],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success],
                    mermaid_check: None,
                },
                // Success
                GameTurn {
                    suggestion: vec![Role::Good, Role::Good2, Role::Mordred],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Success],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Oberon,
                        selection: Role::Mordred,
                        word: Team::Good,
                    }),
                },
                // Reject
                GameTurn {
                    suggestion: vec![Role::Good, Role::Good2, Role::Mordred],
                    team_votes: vec![TeamVote::Reject; 7],
                    try_count: 2,
                    mission_votes: vec![],
                    mermaid_check: None,
                },
                // Fail
                GameTurn {
                    suggestion: vec![Role::Good, Role::Merlin, Role::Mordred],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Fail],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Mordred,
                        selection: Role::Merlin,
                        word: Team::Bad,
                    }),
                },
                // Fail
                GameTurn {
                    suggestion: vec![Role::Good, Role::Merlin, Role::Mordred, Role::Morgen],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Fail, MissionVote::Fail],
                    mermaid_check: Some(MermaidCheck {
                        holder: Role::Merlin,
                        selection: Role::Good,
                        word: Team::Good,
                    }),
                },
                // Fail
                GameTurn {
                    suggestion: vec![Role::Percival, Role::Merlin, Role::Mordred, Role::Morgen],
                    team_votes: vec![TeamVote::Approve; 7],
                    try_count: 1,
                    mission_votes: vec![MissionVote::Success, MissionVote::Success, MissionVote::Fail, MissionVote::Fail],
                    mermaid_check: None, // Last turn, no mermaid check
                }
            ],
            merlin_check: None, // Bad wins, no Merlin check
            expected_game_result: GameResult::BadWins, // 3 fails
        };

        run_test_game(expected).await;
    }
}