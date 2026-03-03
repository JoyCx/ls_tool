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

// Default terminal dimensions when terminal size cannot be detected
const DEFAULT_TERM_WIDTH: usize = 80;
const DEFAULT_TERM_HEIGHT: usize = 24;

// Bytes per kilobyte (binary, not decimal)
const BYTES_PER_KILOBYTE: u64 = 1024;

// Threshold for human-readable size formatting (1024 bytes = 1 KB)
const SIZE_UNIT_THRESHOLD: f64 = 1024.0;

pub fn render(entries: Vec<FileEntry>, args: &Args) -> io::Result<()> {
    let mut buffer = String::new();

    if args.dired {
        print_dired(&entries, &mut buffer);
        print!("{}", buffer);
        return Ok(());
    }

    let use_color = should_use_color(args);

    // If inode, size, or author flags are set, use single-entry format
    if args.one || args.inode || args.size || args.author || args.long {
        for e in &entries {
            render_entry(e, args, use_color, &mut buffer);
        }
    } else if args.columns && !args.across {
        render_columns(&entries, args, use_color, &mut buffer);
    } else if args.across {
        render_across(&entries, args, use_color, &mut buffer);
    } else {
        render_grid(&entries, args, use_color, &mut buffer)?;
    }
    print!("{}", buffer);
    Ok(())
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

pub fn print_dired(entries: &[FileEntry], buffer: &mut String) {
    // First pass: collect byte positions as we build the file list
    let mut positions = Vec::new();

    for entry in entries {
        // Track the byte position where this entry's name starts
        let start_pos = buffer.len();

        // Write the entry (filename + indicator)
        writeln!(buffer, "{}{}", entry.display_name, entry.indicator).ok();

        // Track the byte position where this entry ends
        let end_pos = buffer.len();
        positions.push((start_pos, end_pos));
    }

    // Write the dired markers with byte positions for each entry
    // Format: "  //DIRED// start1 end1 start2 end2 ... //DIRED//"
    write!(buffer, "  //DIRED// ").ok();
    for (start, end) in positions {
        write!(buffer, "{} {} ", start, end).ok();
    }
    writeln!(
        buffer,
        "//DIRED//\n  //DIRED-OPTIONS// --dired //DIRED-OPTIONS//"
    )
    .ok();
}

pub fn should_use_color(args: &Args) -> bool {
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

pub fn render_entry(e: &FileEntry, args: &Args, use_color: bool, buffer: &mut String) {
    if args.long {
        render_long_format(e, args, use_color, buffer);
    } else if args.inode {
        writeln!(buffer, "{:10} {}{}", e.inode, e.display_name, e.indicator).ok();
    } else if args.size {
        let size_str = if args.human_readable {
            format_size_human(e.size)
        } else {
            format_size(e.size, &args.block_size)
        };
        writeln!(buffer, "{:>8} {}{}", size_str, e.display_name, e.indicator).ok();
    } else if args.author {
        writeln!(buffer, "{} {}{}", e.owner, e.display_name, e.indicator).ok();
    } else {
        let name_styled = style_name(e, use_color);
        writeln!(buffer, "{}{}", name_styled, e.indicator).ok();
    }
}

pub fn render_long_format(e: &FileEntry, args: &Args, use_color: bool, buffer: &mut String) {
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

    if args.inode {
        write!(buffer, "{:10} ", e.inode).ok();
    }

    write!(buffer, "{} ", e.permissions).ok();

    write!(buffer, "{:>2} ", 1).ok();

    if args.omit_group {
        write!(buffer, "{:<15} ", owner_str).ok();
    } else {
        write!(buffer, "{:<8} ", owner_str).ok();
        if !group_str.is_empty() {
            write!(buffer, "{:<8} ", group_str).ok();
        }
    }

    if args.size || args.long {
        write!(buffer, "{:>8} ", size_str).ok();
    }

    write!(buffer, "{} ", time_str).ok();

    let name_styled = style_name(e, use_color);

    if e.is_symlink {
        match fs::read_link(&e.path) {
            Ok(target) => {
                let target_str = target.to_string_lossy();
                if use_color {
                    writeln!(
                        buffer,
                        "{}{} -> {}",
                        name_styled,
                        e.indicator,
                        target_str.cyan()
                    )
                    .ok();
                } else {
                    writeln!(buffer, "{}{} -> {}", name_styled, e.indicator, target_str).ok();
                }
            }
            Err(_) => {
                writeln!(buffer, "{}{} -> [broken symlink]", name_styled, e.indicator).ok();
            }
        }
    } else {
        writeln!(buffer, "{}{}", name_styled, e.indicator).ok();
    }
}

pub fn style_name(e: &FileEntry, use_color: bool) -> String {
    let base = e.display_name.clone();
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

pub fn render_columns(entries: &[FileEntry], args: &Args, use_color: bool, buffer: &mut String) {
    // Get terminal width from the terminal, or use default if unavailable
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(DEFAULT_TERM_WIDTH);

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    // Calculate max width needed for any entry (name + indicator + padding)
    let entry_max_width = entries
        .iter()
        .map(|e| e.display_name.len() + e.indicator.len())
        .max()
        .unwrap_or(0);
    let max_len = entry_max_width + 2; // Add 2 spaces of padding

    // Calculate how many columns fit in terminal width
    let cols = (width / max_len).max(1);
    // Calculate rows needed based on entries and column count
    let rows = entries.len().div_ceil(cols);

    // Render grid: iterate by rows, then columns (column-major order)
    for row in 0..rows {
        for col in 0..cols {
            // Convert (col, row) to linear index: col * rows + row
            let idx = col * rows + row;
            if let Some(e) = entries.get(idx) {
                let styled = style_name(e, use_color);
                write!(buffer, "{:<width$}", styled, width = max_len).ok();
            }
        }
        writeln!(buffer).ok();
    }
}

pub fn render_across(entries: &[FileEntry], args: &Args, use_color: bool, buffer: &mut String) {
    // Get terminal width from the terminal, or use default if unavailable
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(DEFAULT_TERM_WIDTH);

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    // Calculate max width needed for any entry (name + indicator + padding)
    let entry_max_width = entries
        .iter()
        .map(|e| e.display_name.len() + e.indicator.len())
        .max()
        .unwrap_or(0);
    let max_len = entry_max_width + 2; // Add 2 spaces of padding

    // Calculate how many columns fit in terminal width
    let cols = (width / max_len).max(1);

    // Render entries left-to-right, wrapping to new line after each column
    for (i, e) in entries.iter().enumerate() {
        let styled = style_name(e, use_color);
        let actual_width = e.display_name.len() + e.indicator.len();

        write!(buffer, "{}{}", styled, e.indicator).ok();
        // Add padding between columns (except at line end)
        if (i + 1) % cols != 0 {
            let padding = " ".repeat(max_len.saturating_sub(actual_width));
            write!(buffer, "{}", padding).ok();
        }

        // Write newline when we've completed a full row
        if (i + 1) % cols == 0 {
            writeln!(buffer).ok();
        }
    }

    // Add final newline if last row wasn't complete
    if !entries.len().is_multiple_of(cols) {
        writeln!(buffer).ok();
    }
}

pub fn render_grid(
    entries: &[FileEntry],
    args: &Args,
    use_color: bool,
    buffer: &mut String,
) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    // Get terminal dimensions, defaulting to 80x24 if unavailable
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

    // Calculate max width needed for any entry (name + indicator + padding)
    let entry_max_width = entries
        .iter()
        .map(|e| e.display_name.len() + e.indicator.len())
        .max()
        .unwrap_or(0);
    let max_len = entry_max_width + 2; // Add 2 spaces of padding

    // If all entries are empty, fall back to single-column layout
    if max_len == 2 {
        for e in entries {
            let styled = style_name(e, use_color);
            writeln!(buffer, "{}{}", styled, e.indicator).ok();
        }
        return Ok(());
    }

    // Calculate grid dimensions: how many columns and rows fit
    let cols = (width / max_len).max(1);
    let rows = entries.len().div_ceil(cols);

    // Create 2D grid structure (column-major: grid[col][row])
    let mut grid = vec![vec![None; rows]; cols];

    // Fill grid with entries in column-major order (top-to-bottom, then left-to-right)
    for (i, e) in entries.iter().enumerate() {
        let col = i / rows; // Which column this entry belongs to
        let row = i % rows; // Which row within that column
        if col < cols {
            grid[col][row] = Some(e);
        }
    }

    // Render grid row-by-row (iterating left-to-right across columns)
    for row in 0..rows {
        for col_vec in grid.iter().take(cols) {
            if let Some(e) = col_vec[row] {
                let styled = style_name(e, use_color);
                let actual_width = e.display_name.len() + e.indicator.len();
                // Pad entries to align columns
                let padding = " ".repeat(max_len.saturating_sub(actual_width));
                write!(buffer, "{}{}{}", styled, e.indicator, padding).ok();
            }
        }
        writeln!(buffer).ok();
    }

    Ok(())
}

pub fn format_size(size: u64, block_size: &str) -> String {
    // Parse the block size multiplier (e.g., "4K" -> 4096)
    let mult = parse_size_multiplier(block_size);
    if mult > 1 {
        // Divide size by block size multiplier
        let size_f = size as f64 / mult as f64;
        // Show without decimals if it's a whole number, otherwise 1 decimal place
        if size_f.fract() == 0.0 {
            format!("{}", size_f as u64)
        } else {
            format!("{:.1}", size_f)
        }
    } else {
        // No multiplier, show size as-is
        size.to_string()
    }
}

pub fn format_size_human(size: u64) -> String {
    // Standard binary SI units (1 KB = 1024 bytes)
    const UNITS: [&str; 9] = ["B", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let mut size = size as f64;
    let mut unit_idx = 0;

    // Scale up units until size is less than 1024 or we reach the largest unit
    while size >= SIZE_UNIT_THRESHOLD && unit_idx < UNITS.len() - 1 {
        size /= SIZE_UNIT_THRESHOLD;
        unit_idx += 1;
    }

    // Format: bytes as whole number, small values with 1 decimal, large values as whole
    if unit_idx == 0 {
        format!("{}", size as u64) // Bytes
    } else if size < 10.0 {
        format!("{:.1}{}", size, UNITS[unit_idx]) // 1 decimal for clarity (e.g., 2.5G)
    } else {
        format!("{:.0}{}", size, UNITS[unit_idx]) // Whole number for large values
    }
}

pub fn parse_size_multiplier(s: &str) -> u64 {
    // Parse block size strings like "4K", "2M", "1G", etc.
    // Returns the total byte multiplier (e.g., "4K" -> 4096)
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return 1;
    }

    // Handle pure text ("K", "MB", etc.) - default to 1x multiplier
    if s.chars().all(|c| c.is_alphabetic()) {
        return match s.as_str() {
            "K" | "KB" => BYTES_PER_KILOBYTE,
            "M" | "MB" => BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE,
            "G" | "GB" => BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE,
            "T" | "TB" => {
                BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE
            }
            _ => 1,
        };
    }

    // Parse numeric prefix (e.g., "4" from "4K")
    let num_part: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    // Parse suffix (e.g., "K" from "4K")
    let suffix: String = s.chars().skip_while(|c| c.is_ascii_digit()).collect();

    // Base multiplier from numeric part (defaults to 1)
    let base: u64 = if num_part.is_empty() {
        1
    } else {
        num_part.parse().unwrap_or(1)
    };

    // Unit multiplier from suffix
    let mult = match suffix.as_str() {
        "K" | "KB" => BYTES_PER_KILOBYTE,
        "M" | "MB" => BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE,
        "G" | "GB" => BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE,
        "T" | "TB" => {
            BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE
        }
        _ => 1,
    };

    // Total multiplier = base * unit (e.g., "4K" -> 4 * 1024)
    base * mult
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
