use clap::{Parser, ValueEnum};
use std::fmt;
use std::io::IsTerminal;
use std::path::PathBuf;

/// Color output options for ls
#[derive(ValueEnum, Clone, Debug, Default)]
pub enum ColorWhen {
    Always,
    #[default]
    Auto,
    Never,
}

impl fmt::Display for ColorWhen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorWhen::Always => write!(f, "always"),
            ColorWhen::Auto => write!(f, "auto"),
            ColorWhen::Never => write!(f, "never"),
        }
    }
}

impl ColorWhen {
    pub fn is_enabled(&self) -> bool {
        match self {
            ColorWhen::Always => true,
            ColorWhen::Never => false,
            ColorWhen::Auto => std::io::stdout().is_terminal(),
        }
    }
}

/// Parse and validate the optional argument to --classify
fn parse_classify_value(s: &str) -> Result<String, String> {
    match s {
        "never" | "always" => Ok(s.to_string()),
        _ => Err(format!(
            "invalid value '{}' for --classify; expected 'never' or 'always'",
            s
        )),
    }
}

/// Command line arguments for ls
#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Rust ls clone for Windows")]
pub struct Args {
    #[arg(default_value = ".", value_parser = parse_path)]
    pub path: Vec<PathBuf>,

    #[arg(short = 'a', long)]
    pub all: bool,

    #[arg(short = 'A', long)]
    pub almost_all: bool,

    #[arg(long)]
    pub author: bool,

    #[arg(short = 'b', long)]
    pub escape: bool,

    #[arg(long, default_value = "1", value_parser = parse_block_size)]
    pub block_size: String,

    #[arg(short = 'B', long)]
    pub ignore_backups: bool,

    #[arg(short = 'c')]
    pub ctime: bool,

    #[arg(short = 'C')]
    pub columns: bool,

    #[arg(long, default_value_t = ColorWhen::Auto, value_enum)]
    pub color: ColorWhen,

    #[arg(short = 'd', long)]
    pub directory: bool,

    #[arg(short = 'D', long)]
    pub dired: bool,

    #[arg(short = 'f')]
    pub unsorted_all: bool,

    #[arg(short = 'F', long, conflicts_with = "file_type", value_parser = parse_classify_value)]
    pub classify: Option<Option<String>>,

    #[arg(long, conflicts_with = "classify")]
    pub file_type: bool,

    #[arg(short = '1', conflicts_with_all = ["columns", "across"])]
    pub one: bool,

    #[arg(short = 'H', long)]
    pub human_readable: bool,

    #[arg(short = 'i', long)]
    pub inode: bool,

    #[arg(short = 'l', long)]
    pub long: bool,

    #[arg(short = 'n', long, requires = "long")]
    pub numeric_uid_gid: bool,

    #[arg(short = 'o', long, requires = "long")]
    pub omit_group: bool,

    #[arg(short = 'q', long = "hide-control-chars")]
    pub hide_control_chars: bool,

    #[arg(short = 'r', long)]
    pub reverse: bool,

    #[arg(short = 'R', long)]
    pub recursive: bool,

    #[arg(short = 's', long, conflicts_with = "human_readable")]
    pub size: bool,

    #[arg(short = 'S')]
    pub sort_size: bool,

    #[arg(short = 't')]
    pub sort_time: bool,

    #[arg(short = 'u', long = "atime")]
    pub atime: bool,

    #[arg(short = 'U')]
    pub no_sort: bool,

    #[arg(short = 'v')]
    pub version_sort: bool,

    #[arg(short = 'w', long, default_value_t = 0)]
    pub width: usize,

    #[arg(short = 'x')]
    pub across: bool,

    #[arg(long, default_value = "locale")]
    pub time_style: String,

    #[arg(long)]
    pub show_control_chars: bool,

    #[arg(
        long, 
        value_parser = ["literal", "shell", "shell-always", "c", "escape"],
        help = "Control how file names are quoted"
    )]
    pub quoting_style: Option<String>,
}

/// Parse a path string into a PathBuf
pub fn parse_path(s: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(s))
}

/// Parse and validate block size argument
pub fn parse_block_size(s: &str) -> Result<String, String> {
    let s = s.to_uppercase();
    if s.is_empty() {
        return Err("Block size cannot be empty".to_string());
    }
    let valid_suffixes = [
        "", "K", "KB", "KIB", "KI", "M", "MB", "MIB", "MI", "G", "GB", "GIB", "GI", "T", "TB",
        "TIB", "TI",
    ];
    let (num_part, suffix_part): (String, String) = s.chars().partition(|c| c.is_ascii_digit());
    if num_part.is_empty() && !valid_suffixes.contains(&suffix_part.as_str()) {
        return Err(format!("Invalid block size suffix: {}", suffix_part));
    }
    Ok(s)
}