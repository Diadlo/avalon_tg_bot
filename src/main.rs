mod game;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut g, _cli) = game::Game::setup(5);
    g.start().await?;
    Ok(())
}
