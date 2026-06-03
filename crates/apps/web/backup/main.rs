fn main() {
    if let Err(error) = run_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_from_env() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if web::is_help_request(&args) {
        print!("{}", web::HELP_TEXT);
        return Ok(());
    }
    let config = web::parse_args(args)?;
    web::run_server(config)
}
