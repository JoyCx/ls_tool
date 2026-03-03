use chrono::{DateTime, Local};
use colored::*;
use is_terminal::IsTerminal;
use std::cmp::Ordering;
use std::fmt::Write;
use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;
use terminal_size::{Height, Width, terminal_size};

use crate::{Args, ColorWhen, FileEntry, is_executable};

const DEFAULT_TERM_WIDTH: usize = 80;
const DEFAULT_TERM_HEIGHT: usize = 24;
const BYTES_PER_KILOBYTE: u64 = 1024;
const SIZE_UNIT_THRESHOLD: f64 = BYTES_PER_KILOBYTE as f64;

pub fn render(
    entries: Vec<FileEntry>,
    args: &Args,
    mut dired_offsets: Option<&mut Vec<(usize, usize)>>,
) -> io::Result<()> {
    let mut buffer = String::new();
    let use_color = should_use_color(args);

    if args.long {
        render_long_entries(&entries, args, use_color, &mut buffer, dired_offsets);
    } else if args.one || args.inode || args.size || args.author {
        for e in &entries {
            render_entry(
                e,
                args,
                use_color,
                &mut buffer,
                dired_offsets.as_deref_mut(),
            );
        }
    } else if args.columns && !args.across {
        render_columns(&entries, args, use_color, &mut buffer, dired_offsets);
    } else if args.across {
        render_across(&entries, args, use_color, &mut buffer, dired_offsets);
    } else {
        render_grid(&entries, args, use_color, &mut buffer, dired_offsets)?;
    }

    print!("{}", buffer);
    Ok(())
}

pub fn append_dired_footer(buffer: &mut String, offsets: Vec<(usize, usize)>) {
    write!(buffer, "  //DIRED//").ok();
    for (start, end) in offsets {
        write!(buffer, " {} {}", start, end).ok();
    }
    writeln!(buffer, "\n  //DIRED-OPTIONS// --dired //DIRED-OPTIONS//").ok();
}

pub fn version_cmp(a: &str, b: &str) -> Ordering {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    let a_version = extract_version_number(&a_lower);
    let b_version = extract_version_number(&b_lower);

    match (a_version, b_version) {
        (Some(av), Some(bv)) => av.cmp(&bv).then_with(|| a_lower.cmp(&b_lower)),
        _ => a_lower.cmp(&b_lower),
    }
}

pub fn extract_version_number(s: &str) -> Option<u64> {
    let numbers: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if !numbers.is_empty() {
        numbers.parse().ok()
    } else {
        None
    }
}

pub fn should_use_color(args: &Args) -> bool {
    if args.dired {
        return false; // dired requires plain text
    }
    match args.color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => std::io::stdout().is_terminal(),
    }
}

pub fn get_indicator(
    path: &Path,
    is_symlink: bool,
    is_dir: bool,
    classify_when: &str,
    file_type_only: bool,
) -> String {
    if classify_when == "never" {
        return String::new();
    }

    if file_type_only {
        if is_symlink {
            return "@".to_string();
        } else if is_dir {
            return "/".to_string();
        }
        return String::new();
    }

    if is_symlink {
        "@".to_string()
    } else if is_dir {
        "/".to_string()
    } else if is_executable(path) {
        "*".to_string()
    } else {
        String::new()
    }
}

pub fn render_entry(
    e: &FileEntry,
    args: &Args,
    use_color: bool,
    buffer: &mut String,
    mut offsets: Option<&mut Vec<(usize, usize)>>,
) {
    if args.long {
        return; // handled separately
    }

    // If multiple flags are present, they are printed in order: inode, size, author, name
    if args.inode {
        write!(buffer, "{:10} ", e.inode).ok();
    }
    if args.size {
        let size_bytes = e.allocated_bytes; // use allocated size for -s
        let size_str = if args.human_readable {
            format_size_human(size_bytes)
        } else {
            format_size(size_bytes, &args.block_size)
        };
        write!(buffer, "{:>8} ", size_str).ok();
    }
    if args.author {
        write!(buffer, "{} ", e.owner).ok();
    }

    let name_start = offsets.as_ref().map(|_| buffer.len());
    let name_styled = style_name(e, args, use_color);
    write!(buffer, "{}", name_styled).ok();
    if let Some(start) = name_start {
        offsets.as_mut().unwrap().push((start, buffer.len()));
    }
    writeln!(buffer, "{}", e.indicator).ok();
}

/// Render all entries in long format with dynamically sized columns.
fn render_long_entries(
    entries: &[FileEntry],
    args: &Args,
    use_color: bool,
    buffer: &mut String,
    mut offsets: Option<&mut Vec<(usize, usize)>>,
) {
    // Compute maximum widths
    let mut max_inode = 0;
    let mut max_blocks = 0;
    let mut max_nlink = 0;
    let mut max_size = 0;
    let mut max_owner = 0;
    let mut max_group = 0;

    for e in entries {
        if args.inode {
            let inode_str = e.inode.to_string();
            max_inode = max_inode.max(inode_str.len());
        }

        if args.size {
            let blocks_str = if args.human_readable {
                format_size_human(e.allocated_bytes)
            } else {
                format_size(e.allocated_bytes, &args.block_size)
            };
            max_blocks = max_blocks.max(blocks_str.len());
        }

        let nlink_str = e.nlink.to_string();
        max_nlink = max_nlink.max(nlink_str.len());

        let size_str = if args.human_readable {
            format_size_human(e.size)
        } else {
            format_size(e.size, &args.block_size)
        };
        max_size = max_size.max(size_str.len());

        let owner_str = if args.numeric_uid_gid {
            if !e.owner_sid.is_empty() {
                e.owner_sid.clone()
            } else {
                "0".to_string()
            }
        } else if !e.owner.is_empty() {
            e.owner.clone()
        } else {
            "Unknown".to_string()
        };
        max_owner = max_owner.max(owner_str.len());

        if !args.omit_group {
            let group_str = if args.numeric_uid_gid {
                if !e.group_sid.is_empty() {
                    e.group_sid.clone()
                } else {
                    "0".to_string()
                }
            } else if !e.group.is_empty() {
                e.group.clone()
            } else {
                "Unknown".to_string()
            };
            max_group = max_group.max(group_str.len());
        }
    }

    // Render each entry
    for e in entries {
        render_long_format(
            e,
            args,
            use_color,
            buffer,
            offsets.as_deref_mut(),
            max_inode,
            max_blocks,
            max_nlink,
            max_size,
            max_owner,
            max_group,
        );
    }
}

/// Render a single entry in long format using precomputed maximum widths.
#[allow(clippy::too_many_arguments)]
fn render_long_format(
    e: &FileEntry,
    args: &Args,
    use_color: bool,
    buffer: &mut String,
    mut offsets: Option<&mut Vec<(usize, usize)>>,
    max_inode: usize,
    max_blocks: usize,
    max_nlink: usize,
    max_size: usize,
    max_owner: usize,
    max_group: usize,
) {
    let time_to_use = if args.ctime {
        e.created
    } else if args.atime {
        e.accessed
    } else {
        e.modified
    };
    let time_str = format_time(time_to_use, &args.time_style);

    let size_str = if args.human_readable {
        format_size_human(e.size)
    } else {
        format_size(e.size, &args.block_size)
    };

    let owner_str = if args.numeric_uid_gid {
        if !e.owner_sid.is_empty() {
            e.owner_sid.clone()
        } else {
            "0".to_string()
        }
    } else if !e.owner.is_empty() {
        e.owner.clone()
    } else {
        "Unknown".to_string()
    };

    let group_str = if args.numeric_uid_gid {
        if !e.group_sid.is_empty() {
            e.group_sid.clone()
        } else {
            "0".to_string()
        }
    } else if !e.group.is_empty() && !args.omit_group {
        e.group.clone()
    } else if !args.omit_group {
        "Unknown".to_string()
    } else {
        String::new()
    };

    // Block count (if -s)
    if args.size {
        let blocks_str = if args.human_readable {
            format_size_human(e.allocated_bytes)
        } else {
            format_size(e.allocated_bytes, &args.block_size)
        };
        write!(buffer, "{:>width$} ", blocks_str, width = max_blocks).ok();
    }

    if args.inode {
        write!(buffer, "{:>width$} ", e.inode, width = max_inode).ok();
    }

    write!(buffer, "{} ", e.permissions).ok();
    write!(buffer, "{:>width$} ", e.nlink, width = max_nlink).ok();

    if args.omit_group {
        write!(buffer, "{:<width$} ", owner_str, width = max_owner).ok();
    } else {
        write!(buffer, "{:<width$} ", owner_str, width = max_owner).ok();
        if !group_str.is_empty() {
            write!(buffer, "{:<width$} ", group_str, width = max_group).ok();
        }
    }

    write!(buffer, "{:>width$} ", size_str, width = max_size).ok();
    write!(buffer, "{} ", time_str).ok();

    let name_start = offsets.as_ref().map(|_| buffer.len());
    let name_styled = style_name(e, args, use_color);
    write!(buffer, "{}", name_styled).ok();
    if let Some(start) = name_start {
        offsets.as_mut().unwrap().push((start, buffer.len()));
    }

    if e.is_symlink {
        match fs::read_link(&e.path) {
            Ok(target) => {
                let target_str = target.to_string_lossy();
                let arrow = if use_color {
                    format!(" -> {}", target_str.cyan())
                } else {
                    format!(" -> {}", target_str)
                };
                writeln!(buffer, "{}{}", e.indicator, arrow).ok();
            }
            Err(_) => {
                writeln!(buffer, "{} -> [broken symlink]", e.indicator).ok();
            }
        }
    } else {
        writeln!(buffer, "{}", e.indicator).ok();
    }
}

pub fn style_name(e: &FileEntry, args: &Args, use_color: bool) -> String {
    let base = format_file_name(e, args);
    if !use_color {
        return base;
    }

    if e.is_symlink {
        base.cyan().bold().to_string()
    } else if e.is_dir {
        base.blue().bold().to_string()
    } else if is_executable(&e.path) {
        base.green().to_string()
    } else {
        base.normal().to_string()
    }
}

pub fn render_columns(
    entries: &[FileEntry],
    args: &Args,
    use_color: bool,
    buffer: &mut String,
    mut offsets: Option<&mut Vec<(usize, usize)>>,
) {
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(DEFAULT_TERM_WIDTH);

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    let entry_max_width = entries
        .iter()
        .map(|e| format_file_name(e, args).len() + e.indicator.len())
        .max()
        .unwrap_or(0);
    let max_len = entry_max_width + 2;

    let cols = (width / max_len).max(1);
    let rows = entries.len().div_ceil(cols);

    for row in 0..rows {
        for col in 0..cols {
            let idx = col * rows + row;
            if let Some(e) = entries.get(idx) {
                let name_start = offsets.as_ref().map(|_| buffer.len());
                let styled = style_name(e, args, use_color);
                write!(buffer, "{}", styled).ok();
                if let Some(start) = name_start {
                    offsets.as_mut().unwrap().push((start, buffer.len()));
                }
                write!(buffer, "{}", e.indicator).ok();
                let actual_width = format_file_name(e, args).len() + e.indicator.len();
                let padding = " ".repeat(max_len.saturating_sub(actual_width));
                write!(buffer, "{}", padding).ok();
            }
        }
        writeln!(buffer).ok();
    }
}

pub fn render_across(
    entries: &[FileEntry],
    args: &Args,
    use_color: bool,
    buffer: &mut String,
    mut offsets: Option<&mut Vec<(usize, usize)>>,
) {
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(DEFAULT_TERM_WIDTH);

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    let entry_max_width = entries
        .iter()
        .map(|e| format_file_name(e, args).len() + e.indicator.len())
        .max()
        .unwrap_or(0);
    let max_len = entry_max_width + 2;
    let cols = (width / max_len).max(1);

    for (i, e) in entries.iter().enumerate() {
        let name_start = offsets.as_ref().map(|_| buffer.len());
        let styled = style_name(e, args, use_color);
        let name_len = format_file_name(e, args).len() + e.indicator.len();

        write!(buffer, "{}{}", styled, e.indicator).ok();
        if let Some(start) = name_start {
            offsets.as_mut().unwrap().push((start, buffer.len()));
        }

        if (i + 1) % cols != 0 {
            let padding = " ".repeat(max_len.saturating_sub(name_len));
            write!(buffer, "{}", padding).ok();
        }

        if (i + 1) % cols == 0 {
            writeln!(buffer).ok();
        }
    }

    if !entries.len().is_multiple_of(cols) {
        writeln!(buffer).ok();
    }
}

pub fn render_grid(
    entries: &[FileEntry],
    args: &Args,
    use_color: bool,
    buffer: &mut String,
    mut offsets: Option<&mut Vec<(usize, usize)>>,
) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let term_size = terminal_size();
    let (term_width, _term_height) = match term_size {
        Some((Width(w), Height(h))) => (w as usize, h as usize),
        None => (DEFAULT_TERM_WIDTH, DEFAULT_TERM_HEIGHT),
    };

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    let entry_max_width = entries
        .iter()
        .map(|e| format_file_name(e, args).len() + e.indicator.len())
        .max()
        .unwrap_or(0);
    let max_len = entry_max_width + 2;

    if max_len == 2 {
        for e in entries {
            let styled = style_name(e, args, use_color);
            writeln!(buffer, "{}{}", styled, e.indicator).ok();
        }
        return Ok(());
    }

    let cols = (width / max_len).max(1);
    let rows = entries.len().div_ceil(cols);

    let mut grid = vec![vec![None; rows]; cols];
    for (i, e) in entries.iter().enumerate() {
        let col = i / rows;
        let row = i % rows;
        if col < cols {
            grid[col][row] = Some(e);
        }
    }

    for row in 0..rows {
        for col in 0..cols {
            let idx = col * rows + row;
            if let Some(e) = entries.get(idx) {
                let name_start = offsets.as_ref().map(|_| buffer.len());
                let styled = style_name(e, args, use_color);
                let actual_width = format_file_name(e, args).len() + e.indicator.len();

                write!(buffer, "{}{}", styled, e.indicator).ok();
                if let Some(start) = name_start {
                    offsets.as_mut().unwrap().push((start, buffer.len()));
                }

                let padding = " ".repeat(max_len.saturating_sub(actual_width));
                write!(buffer, "{}", padding).ok();
            }
        }
        writeln!(buffer).ok();
    }

    Ok(())
}

pub fn format_size(size: u64, block_size: &str) -> String {
    let mult = parse_size_multiplier(block_size);
    if mult > 1 {
        let size_f = size as f64 / mult as f64;
        if size_f.fract() == 0.0 {
            format!("{}", size_f as u64)
        } else {
            format!("{:.1}", size_f)
        }
    } else {
        size.to_string()
    }
}

pub fn format_size_human(size: u64) -> String {
    const UNITS: [&str; 9] = ["B", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let mut size = size as f64;
    let mut unit_idx = 0;

    while size >= SIZE_UNIT_THRESHOLD && unit_idx < UNITS.len() - 1 {
        size /= SIZE_UNIT_THRESHOLD;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{}", size as u64)
    } else if size < 10.0 {
        format!("{:.1}{}", size, UNITS[unit_idx])
    } else {
        format!("{:.0}{}", size, UNITS[unit_idx])
    }
}

pub fn parse_size_multiplier(s: &str) -> u64 {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return 1;
    }

    // If it's just a suffix (no digits), treat as multiplier
    if s.chars().all(|c| c.is_alphabetic()) {
        return suffix_multiplier(&s);
    }

    let num_part: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let suffix: String = s.chars().skip_while(|c| c.is_ascii_digit()).collect();

    let base: u64 = if num_part.is_empty() {
        1
    } else {
        num_part.parse().unwrap_or(1)
    };

    base * suffix_multiplier(&suffix)
}

fn suffix_multiplier(suffix: &str) -> u64 {
    match suffix {
        "K" | "KI" | "KIB" => BYTES_PER_KILOBYTE,
        "KB" => 1000,
        "M" | "MI" | "MIB" => BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE,
        "MB" => 1000 * 1000,
        "G" | "GI" | "GIB" => BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE,
        "GB" => 1000 * 1000 * 1000,
        "T" | "TI" | "TIB" => {
            BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE
        }
        "TB" => 1000u64 * 1000 * 1000 * 1000,
        _ => 1,
    }
}

pub fn format_time(t: SystemTime, style: &str) -> String {
    let dt: DateTime<Local> = t.into();

    match style {
        "full-iso" => dt.format("%Y-%m-%d %H:%M:%S.%f %z").to_string(),
        "long-iso" => dt.format("%Y-%m-%d %H:%M").to_string(),
        "iso" => dt.format("%Y-%m-%d").to_string(),
        "locale" => dt.format("%b %e %H:%M").to_string(),
        custom if custom.starts_with('+') => {
            let format_str = &custom[1..];
            dt.format(format_str).to_string()
        }
        _ => dt.format("%b %e %H:%M").to_string(),
    }
}

fn format_file_name(e: &FileEntry, args: &Args) -> String {
    let name = &e.name;
    let style =
        args.quoting_style
            .as_deref()
            .unwrap_or(if args.escape { "escape" } else { "literal" });

    // Determine if we should hide control chars
    let hide_control = if args.show_control_chars {
        false
    } else if args.hide_control_chars {
        true
    } else {
        // Default: hide if stdout is a terminal
        !std::io::stdout().is_terminal()
    };

    match style {
        "c" => format!("\"{}\"", crate::util::escape_non_graphic(name)),
        "escape" => crate::util::escape_non_graphic(name),
        "shell" | "shell-always" => format!("'{}'", name.replace('\'', "'\\''")),
        _ => {
            if hide_control {
                crate::util::hide_control_chars(name)
            } else {
                name.to_string()
            }
        }
    }
}
