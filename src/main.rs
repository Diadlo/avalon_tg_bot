mod game;

fn main() {
    let g = game::Game::setup(5);
    g.start();
}
