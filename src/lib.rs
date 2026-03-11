use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub mod args;
pub mod formatting;
pub mod util;
pub mod windows_util;

pub use args::{Args, ColorWhen};
pub use formatting::{get_indicator, render, version_cmp};
pub use util::{
    cache_get_or_compute, cache_get_or_compute_sync, escape_non_graphic, hide_control_chars,
    is_backup_file, is_executable,
};

// Standard Windows file-attribute bitmask constants.
// These are stable, well-documented values from the Win32 SDK; no FFI needed.
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0000_0020;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

pub use windows_util::{
    calculate_inode, get_allocated_size, get_file_attributes_windows, get_nlink,
    get_owner_and_group, get_windows_permissions,
};

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_hidden: bool,
    pub is_system: bool,
    pub is_readonly: bool,
    pub size: u64,
    pub allocated_bytes: u64, // actual disk usage in bytes (for -s)
    pub modified: SystemTime,
    pub created: SystemTime,
    pub accessed: SystemTime,
    pub owner: String,
    pub owner_sid: String,
    pub group: String,
    pub group_sid: String,
    pub indicator: String,
    pub permissions: String,
    pub file_attributes: u32,
    pub inode: u64,
    pub nlink: u32,
    pub path: PathBuf,
}

pub fn run(mut args: Args) -> io::Result<i32> {
    // --author implies long format (GNU behaviour)
    if args.author && !args.long {
        args.long = true;
    }

    // -f overrides: -a, -U, and disables all extra formatting
    if args.unsorted_all {
        args.all = true;
        args.no_sort = true;
        args.long = false;
        args.size = false;
        args.author = false;
        args.inode = false;
        args.ctime = false;
        args.atime = false;
        args.escape = false;
        args.human_readable = false;
    }

    let classify_when = match &args.classify {
        Some(Some(when)) => when.clone(),
        Some(None) => "always".to_string(),
        None => {
            if args.file_type {
                "always".to_string()
            } else {
                "never".to_string()
            }
        }
    };

    let mut exit_code = 0;
    let paths = if args.path.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.path.clone()
    };

    let mut first_path = true;
    let mut dired_offsets = if args.dired { Some(Vec::new()) } else { None };

    for path in paths.iter() {
        if paths.len() > 1 {
            if !first_path {
                println!();
            }
            println!("{}:", path.display());
            first_path = false;
        }

        let mut entries = Vec::new();

        if args.recursive {
            if args.directory {
                match process_path(path, &args, &classify_when) {
                    Ok(entry) => entries.push(entry),
                    Err(e) => {
                        eprintln!("ls: cannot access '{}': {}", path.display(), e);
                        exit_code = 1;
                        continue;
                    }
                }
            } else {
                let result: Result<Vec<(usize, PathBuf, Vec<FileEntry>)>, io::Error> =
                    collect_recursive_entries(path, &args, &classify_when, 0);
                match result {
                    Ok(recursive_entries) => {
                        for (depth, dir_path, dir_entries) in recursive_entries {
                            if depth > 0 {
                                println!();
                                println!("{}:", dir_path.display());
                            }
                            if !dir_entries.is_empty() {
                                render(dir_entries, &args, dired_offsets.as_mut())?;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("ls: cannot access '{}': {}", path.display(), e);
                        exit_code = 1;
                    }
                }
                continue;
            }
        } else if args.directory {
            match process_path(path, &args, &classify_when) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    eprintln!("ls: cannot access '{}': {}", path.display(), e);
                    exit_code = 1;
                    continue;
                }
            }
        } else {
            match fs::read_dir(path) {
                Ok(read_dir) => {
                    if args.all {
                        if let Ok(e) = process_path(&path.join("."), &args, &classify_when) {
                            entries.push(e);
                        }
                        if let Ok(e) = process_path(&path.join(".."), &args, &classify_when) {
                            entries.push(e);
                        }
                    }

                    for entry in read_dir.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();

                        if args.almost_all && (name == "." || name == "..") {
                            continue;
                        }
                        if !args.all && !args.almost_all && name.starts_with('.') {
                            continue;
                        }
                        if args.ignore_backups && is_backup_file(&name) {
                            continue;
                        }
                        match process_path(&entry.path(), &args, &classify_when) {
                            Ok(e) => {
                                if !args.all && !args.almost_all && e.is_hidden {
                                    continue;
                                }
                                entries.push(e);
                            }
                            Err(e) => {
                                eprintln!("ls: cannot access '{}': {}", entry.path().display(), e);
                                exit_code = 1;
                            }
                        }
                    }
                }
                Err(e) => match process_path(path, &args, &classify_when) {
                    Ok(entry) => entries.push(entry),
                    Err(_) => {
                        eprintln!("ls: cannot access '{}': {}", path.display(), e);
                        exit_code = 1;
                        continue;
                    }
                },
            }
        }

        if !args.no_sort {
            sort_entries(&mut entries, &args);
        }

        render(entries, &args, dired_offsets.as_mut())?;
    }

    // Print dired footer if needed
    if let Some(offsets) = dired_offsets {
        let mut footer = String::new();
        formatting::append_dired_footer(&mut footer, offsets);
        print!("{}", footer);
    }

    Ok(exit_code)
}

pub fn collect_recursive_entries(
    path: &Path,
    args: &Args,
    classify_when: &str,
    depth: usize,
) -> io::Result<Vec<(usize, PathBuf, Vec<FileEntry>)>> {
    let mut result = Vec::new();
    let mut current_dir_entries = Vec::new();

    match fs::read_dir(path) {
        Ok(read_dir) => {
            if args.all {
                if let Ok(e) = process_path(&path.join("."), args, classify_when) {
                    current_dir_entries.push(e);
                }
                if let Ok(e) = process_path(&path.join(".."), args, classify_when) {
                    current_dir_entries.push(e);
                }
            }

            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();

                if args.almost_all && (name == "." || name == "..") {
                    continue;
                }
                if !args.all && !args.almost_all && name.starts_with('.') {
                    continue;
                }
                if args.ignore_backups && is_backup_file(&name) {
                    continue;
                }

                match process_path(&entry.path(), args, classify_when) {
                    Ok(e) => {
                        if !args.all && !args.almost_all && e.is_hidden {
                            continue;
                        }
                        current_dir_entries.push(e);
                    }
                    Err(e) => {
                        eprintln!("ls: cannot access '{}': {}", entry.path().display(), e);
                    }
                }
            }
        }
        Err(e) => {
            return Err(e);
        }
    }

    if !current_dir_entries.is_empty() {
        if !args.no_sort {
            sort_entries(&mut current_dir_entries, args);
        }
        result.push((depth, path.to_path_buf(), current_dir_entries));
    }

    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let metadata = fs::metadata(&path);

            if let Ok(metadata) = metadata
                && metadata.is_dir()
            {
                let name = entry.file_name().to_string_lossy().to_string();

                if (args.almost_all && (name == "." || name == ".."))
                    || (!args.all && !args.almost_all && name.starts_with('.'))
                {
                    continue;
                }

                match collect_recursive_entries(&path, args, classify_when, depth + 1) {
                    Ok(mut sub_results) => {
                        result.append(&mut sub_results);
                    }
                    Err(e) => {
                        eprintln!("ls: cannot access '{}': {}", path.display(), e);
                    }
                }
            }
        }
    }

    Ok(result)
}

pub fn process_path(path: &Path, args: &Args, classify_when: &str) -> io::Result<FileEntry> {
    let metadata_result = fs::symlink_metadata(path);
    let metadata = match metadata_result {
        Ok(meta) => meta,
        Err(e) => {
            let file_attributes = get_file_attributes_windows(path).unwrap_or(0);
            if file_attributes == 0 {
                return Err(e);
            }
            let file = std::fs::File::open(path)?;
            file.metadata()?
        }
    };

    let name: String = {
        let path_str = path.to_string_lossy();
        if path_str.ends_with("\\.") || path_str.ends_with("/.") {
            ".".to_string()
        } else if path_str.ends_with("\\..") || path_str.ends_with("/..") {
            "..".to_string()
        } else {
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .to_string()
        }
    };

    let file_attributes = get_file_attributes_windows(path).unwrap_or(0);

    // Plain u32 bitmask checks — no `.0` newtype accessor needed.
    let is_symlink = (file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
    let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
    let is_hidden = (file_attributes & FILE_ATTRIBUTE_HIDDEN) != 0;
    let is_system = (file_attributes & FILE_ATTRIBUTE_SYSTEM) != 0;
    let is_readonly = (file_attributes & FILE_ATTRIBUTE_READONLY) != 0;

    let indicator = get_indicator(path, is_symlink, is_dir, classify_when, args.file_type);

    let permissions = get_windows_permissions(path, is_symlink, is_dir, file_attributes);

    let (owner, owner_sid, group, group_sid) = if args.author || args.long {
        get_owner_and_group(path).unwrap_or_else(|_| {
            (
                "Unknown".to_string(),
                "".to_string(),
                "Unknown".to_string(),
                "".to_string(),
            )
        })
    } else {
        (String::new(), String::new(), String::new(), String::new())
    };

    let nlink = if args.long {
        windows_util::get_nlink(path).unwrap_or(1)
    } else {
        1
    };

    let inode = calculate_inode(path).unwrap_or(0);

    // Allocated size (for -s) – compute only if needed
    let allocated_bytes = if args.size {
        windows_util::get_allocated_size(path).unwrap_or(metadata.len())
    } else {
        0
    };

    Ok(FileEntry {
        name,
        is_dir,
        is_symlink,
        is_hidden,
        is_system,
        is_readonly,
        size: metadata.len(),
        allocated_bytes,
        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        created: metadata.created().unwrap_or(SystemTime::UNIX_EPOCH),
        accessed: metadata.accessed().unwrap_or(SystemTime::UNIX_EPOCH),
        owner,
        owner_sid,
        group,
        group_sid,
        indicator,
        permissions,
        file_attributes,
        inode,
        nlink,
        path: path.to_path_buf(),
    })
}

pub fn sort_entries(entries: &mut [FileEntry], args: &Args) {
    if args.no_sort {
        return;
    }

    entries.sort_by(|a, b| {
        let cmp = if args.sort_time {
            let (ta, tb) = if args.ctime {
                (a.created, b.created)
            } else if args.atime {
                (a.accessed, b.accessed)
            } else {
                (a.modified, b.modified)
            };
            tb.cmp(&ta).then_with(|| a.name.cmp(&b.name))
        } else if args.sort_size {
            b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name))
        } else if args.version_sort {
            version_cmp(&a.name, &b.name)
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        };

        if args.reverse { cmp.reverse() } else { cmp }
    });
}
