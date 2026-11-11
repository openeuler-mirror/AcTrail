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
    let config = web::parse_args(args).map_err(|error| format!(
        "load Web configuration or arguments failed: {error}; check --config, --addr and --port; initialize a configuration with actrailctl init"
    ))?;
    let listen_addr = config.listen_addr;
    web::run_server(config).map_err(|error| format!(
        "run Web server at {listen_addr} failed: {error}; check configuration and storage permissions; stop the process using this address or choose another --addr/--port"
    ))
}
