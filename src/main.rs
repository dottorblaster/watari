use anyhow::Result;
use watari::cli::{self, Action};
use watari::config::Env;
use watari::daemon;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let action = match cli::parse(std::env::args().skip(1), &Env::capture()) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("watari: {error}");
            eprintln!("Try 'watari --help' for more information.");
            std::process::exit(2);
        }
    };

    if let Err(error) = execute(action) {
        eprintln!("watari: {error:#}");
        std::process::exit(1);
    }
}

fn execute(action: Action) -> Result<()> {
    match action {
        Action::Run(config) => daemon::run(&config),
        Action::PrintConfig(config) => {
            print!("{}", config.render());
            Ok(())
        }
        Action::Help => {
            print!("{}", cli::USAGE);
            Ok(())
        }
        Action::Version => {
            println!("watari {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
