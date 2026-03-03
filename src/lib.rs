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
    is_executable,
};
pub use windows_util::{
    FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM, calculate_inode,
    get_file_attributes_windows, get_owner_and_group, get_windows_permissions,
};

pub struct FileEntry {
    pub name: String,
    pub display_name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_hidden: bool,
    pub is_system: bool,
    pub is_readonly: bool,
    pub size: u64,
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
    pub path: PathBuf,
}

pub fn run(mut args: Args) -> io::Result<i32> {
    /*
    if user passed -f (unsorted_all):
    force --all
    force --no-sort
    */
    if args.unsorted_all {
        args.all = true;
        args.no_sort = true;
    }

    /*
    CLI input	                classify_when
    --classify=never	        "never"
    --classify (no value)	    "always"
    not provided + --file-type	"always"
    neither provided	        "never"
    */
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

    /*
    exit code starts at 0 (success).
    if no path arguments were given:
    default to "." (current directory).
    */
    let mut exit_code = 0;
    let paths = if args.path.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.path.clone()
    };

    // used to make newline between entries
    let mut first_path = true;
    for path in paths.iter() {
        if paths.len() > 1 {
            if !first_path {
                println!();
            }
            // actual path printing
            println!("{}:", path.display());
            first_path = false;
        }

        // metadata current path
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
                                render(dir_entries, &args)?;
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
                        if args.ignore_backups && name.ends_with('~') {
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

        render(entries, &args)?;
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
                if args.ignore_backups && name.ends_with('~') {
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

    // Special handling for . and ..
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

    let mut display_name = name.clone();

    if args.escape {
        display_name = escape_non_graphic(&display_name);
    } else if args.hide_control_chars && !args.show_control_chars {
        display_name = hide_control_chars(&display_name);
    }

    let file_attributes = get_file_attributes_windows(path).unwrap_or(0);
    let is_symlink = (file_attributes & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0;
    let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
    let is_hidden = (file_attributes & FILE_ATTRIBUTE_HIDDEN.0) != 0;
    let is_system = (file_attributes & FILE_ATTRIBUTE_SYSTEM.0) != 0;
    let is_readonly = (file_attributes & FILE_ATTRIBUTE_READONLY.0) != 0;

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

    let inode = calculate_inode(path).unwrap_or(0);

    Ok(FileEntry {
        name,
        display_name,
        is_dir,
        is_symlink,
        is_hidden,
        is_system,
        is_readonly,
        size: metadata.len(),
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
