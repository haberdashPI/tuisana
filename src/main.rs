use tuisana::{
    app::{debug_log, App},
    asana::client::HttpAsanaClient,
    config::Config,
    ui::runtime::{run_project_list_app, CrosstermKeySource},
};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{backend::CrosstermBackend, Terminal};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_path("tuisana.toml")?;
    let auth = config
        .auth
        .as_ref()
        .ok_or_else(|| std::io::Error::other("missing [auth] config in tuisana.toml"))?;
    if std::env::var_os("TUISANA_DEBUG").is_some() {
        let bind_list = config
            .bind
            .iter()
            .map(|bind| format!("{}={}", bind.key, bind.command))
            .collect::<Vec<_>>()
            .join(", ");
        debug_log(&format!("startup binds: {bind_list}"));
    }
    let client = HttpAsanaClient::from_config(auth)?;
    let mut app = App::with_config_path("tuisana.toml", config, client);
    app.load_projects()?;

    let mut source = CrosstermKeySource;
    let mut stdout = std::io::stdout();
    crossterm::execute!(&mut stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_project_list_app(&mut app, &mut source, &mut terminal);
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result?;
    Ok(())
}
