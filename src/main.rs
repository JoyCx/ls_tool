use clap::Parser;
use ls_tool::{Args, run};
use std::process::exit;

fn main() {
    let args: Args = Args::parse();
    colored::control::set_override(args.color.is_enabled());

    match run(args) {
        Ok(code) => exit(code),
        Err(e) => {
            eprintln!("ls: {}", e);
            exit(2);
        }
    }
}
