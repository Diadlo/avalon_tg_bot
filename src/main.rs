mod game;

#[tokio::main]
async fn main() {
    let mut g = game::Game::setup(5);
    g.start().await;
}
