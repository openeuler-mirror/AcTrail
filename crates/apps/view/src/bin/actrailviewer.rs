fn main() {
    if let Err(error) = run_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_from_env() -> Result<(), String> {
    let invocation = view::parse_invocation(std::env::args().skip(1))?;
    println!("{}", view::render_storage_view(invocation)?);
    Ok(())
}
