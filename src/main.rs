use tuisana::{
    app::App,
    asana::fake::FakeAsanaClient,
    config::Config,
    ui::project_list::render_project_list,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = FakeAsanaClient::with_default_projects();
    let mut app = App::new(Config::default(), client);
    app.load_projects()?;

    let view = render_project_list(&app.projects);
    println!("{}", view.title);
    println!("{}", view.status_line);
    for row in view.rows {
        println!("{row}");
    }
    Ok(())
}
