use tuisana::{
    app::App,
    asana::client::HttpAsanaClient,
    config::Config,
    ui::project_list::render_project_list,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_path("tuisana.toml")?;
    let auth = config
        .auth
        .as_ref()
        .ok_or_else(|| std::io::Error::other("missing [auth] config in tuisana.toml"))?;
    let client = HttpAsanaClient::from_config(auth)?;
    let mut app = App::new(config, client);
    app.load_projects()?;

    let view = render_project_list(&app.projects);
    println!("{}", view.title);
    println!("{}", view.status_line);
    for row in view.rows {
        println!("{row}");
    }
    Ok(())
}
