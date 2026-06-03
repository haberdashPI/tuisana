use tuisana::{app::App, asana::fake::FakeAsanaClient, config::Config};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = FakeAsanaClient::with_default_projects();
    let mut app = App::new(Config::default(), client);
    app.load_projects()?;

    println!("Tuisana initialized with {} project(s).", app.projects.len());
    Ok(())
}
