use clap::Parser;
use ls_tool::{Args, ColorWhen, run};
use std::{io::IsTerminal, process::exit};

fn main() {
    let args = Args::parse();
    let color_enabled = match args.color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => std::io::stdout().is_terminal(),
    };
    if color_enabled {
        colored::control::set_override(true);
    } else {
        colored::control::set_override(false);
    }

    match run(args) {
        Ok(code) => exit(code),
        Err(e) => {
            eprintln!("ls: {}", e);
            exit(2);
        }
    }
}
