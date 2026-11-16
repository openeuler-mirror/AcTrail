//! Run the grey proxy child with configuration received from its owning runtime.
#[path = "../config.rs"]
mod config;
#[path = "../history.rs"]
mod history;
#[path = "../proxy.rs"]
mod proxy;
#[path = "../remote.rs"]
mod remote;
use std::io::{BufRead, Read};
fn main() {
    let result = (|| {
        let mut raw = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut raw)
            .map_err(|e| e.to_string())?;
        let config = config::Config::parse(&raw)?;
        // The owner retains stdin. EOF also closes the child after an abrupt daemon exit.
        std::thread::spawn(|| {
            let mut byte = [0u8; 1];
            let _ = std::io::stdin().read(&mut byte);
            std::process::exit(0);
        });
        proxy::GreyProxy::run(config)
    })();
    if let Err(error) = result {
        eprintln!("grey proxy: {error}");
        std::process::exit(1);
    }
}
