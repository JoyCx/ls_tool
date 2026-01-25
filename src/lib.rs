use chrono::{DateTime, Local};
use clap::{Parser, ValueEnum};
use colored::*;
use is_terminal::IsTerminal;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;
use terminal_size::{Height, Width, terminal_size};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, GetLastError};
use windows::Win32::Security::{
    GROUP_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorGroup,
    GetSecurityDescriptorOwner, LookupAccountSidW, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_SYSTEM, GetFileAttributesW, GetFileInformationByHandle,
};
use windows::core::{PCWSTR, PWSTR};

#[derive(ValueEnum, Clone, Debug, Default)]
pub enum ColorWhen {
    Always,
    #[default]
    Auto,
    Never,
}

//converts enum to string in print formatting
impl fmt::Display for ColorWhen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorWhen::Always => write!(f, "always"),
            ColorWhen::Auto => write!(f, "auto"),
            ColorWhen::Never => write!(f, "never"),
        }
    }
}

//args we use for ls, default value, custom parsing function/auto parsing using enum, and long "--var" vs short "-v"
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

    #[arg(short = 'F', long, conflicts_with = "file_type")]
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

    #[arg(long, default_value = "locale", value_parser = ["locale", "long-iso", "iso", "full-iso"])]
    pub time_style: String,

    #[arg(long)]
    pub show_control_chars: bool,

    #[arg(long, hide = true)]
    pub quoting_style: Option<String>,
}

//parse path from string
pub fn parse_path(s: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(s))
}

//
pub fn parse_block_size(s: &str) -> Result<String, String> {
    /*
    converts input to uppercase.
    so 10k, 10Kb, 10mb all become:
    10K, 10KB, 10MB
    */
    let s = s.to_uppercase();
    // reject empty input
    if s.is_empty() {
        return Err("Block size cannot be empty".to_string());
    }
    // these are the only suffixes allowed for block size
    let valid_suffixes = ["", "K", "KB", "M", "MB", "G", "GB", "T", "TB"];
    // split into digits vs non-digits
    let (num_part, suffix_part): (String, String) = s.chars().partition(|c| c.is_ascii_digit());
    // If there are no digits at all and the suffix is not in the allowed list
    if num_part.is_empty() && !valid_suffixes.contains(&suffix_part.as_str()) {
        return Err(format!("Invalid block size suffix: {}", suffix_part));
    }
    // we have at least an ok suffix / number
    Ok(s)
}

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
            let file_attributes = get_file_attributes_windows(path);
            if file_attributes == 0 {
                return Err(e);
            }
            let file = std::fs::File::open(path)?;
            file.metadata()?
        }
    };

    let name: String = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .to_string();

    let mut display_name = name.clone();

    if args.escape {
        display_name = escape_non_graphic(&display_name);
    } else if args.hide_control_chars && !args.show_control_chars {
        display_name = hide_control_chars(&display_name);
    }

    let file_attributes = get_file_attributes_windows(path);
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

pub fn escape_non_graphic(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            '\n' => "\\n".to_string(),
            '\x1b' => "\\e".to_string(),
            '\x07' => "\\a".to_string(),
            '\x08' => "\\b".to_string(),
            '\x0c' => "\\f".to_string(),
            '\x0b' => "\\v".to_string(),
            '\\' => "\\\\".to_string(),
            c if c.is_control() => format!("\\{:03o}", c as u8),
            _ => c.to_string(),
        })
        .collect()
}

pub fn hide_control_chars(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\t' && c != '\n' && c != '\r' {
                '?'
            } else {
                c
            }
        })
        .collect()
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

pub fn get_file_attributes_windows(path: &Path) -> u32 {
    unsafe {
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let attrs = GetFileAttributesW(PCWSTR(wide_path.as_ptr()));

        if attrs == 0xFFFFFFFF {
            let last_error = GetLastError();
            if last_error != ERROR_ACCESS_DENIED {}
            0
        } else {
            attrs
        }
    }
}

pub fn is_executable(path: &Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_lowercase();
            matches!(
                ext.as_str(),
                "exe" | "bat" | "cmd" | "ps1" | "com" | "msi" | "sh" | "py" | "pl" | "rb" | "js"
            )
        })
        .unwrap_or(false)
}

pub fn get_owner_and_group(path: &Path) -> io::Result<(String, String, String, String)> {
    static SID_CACHE: OnceLock<HashMap<String, (String, String, String, String)>> = OnceLock::new();

    let mut cache = SID_CACHE.get_or_init(HashMap::new).clone();
    let path_str = path.to_string_lossy().to_string();

    if let Some(cached) = cache.get(&path_str) {
        return Ok(cached.clone());
    }

    let result = unsafe {
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut needed = 0u32;

        let security_info = OWNER_SECURITY_INFORMATION.0 | GROUP_SECURITY_INFORMATION.0;

        let result = GetFileSecurityW(
            PCWSTR(wide_path.as_ptr()),
            security_info,
            None,
            0,
            &mut needed,
        );

        if result.0 == 0 && needed == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(5) {
                return Ok((
                    "ACCESS_DENIED".to_string(),
                    "".to_string(),
                    "ACCESS_DENIED".to_string(),
                    "".to_string(),
                ));
            }
            return Err(error);
        }

        if needed == 0 {
            return Ok((
                "Unknown".to_string(),
                "".to_string(),
                "Unknown".to_string(),
                "".to_string(),
            ));
        }

        let mut buf = vec![0u8; needed as usize];

        let result = GetFileSecurityW(
            PCWSTR(wide_path.as_ptr()),
            security_info,
            Some(PSECURITY_DESCRIPTOR(buf.as_mut_ptr() as *mut _)),
            needed,
            &mut needed,
        );

        if result.0 == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut owner_sid = PSID::default();
        let mut owner_defaulted = false.into();
        let result = GetSecurityDescriptorOwner(
            PSECURITY_DESCRIPTOR(buf.as_ptr() as *mut _),
            &mut owner_sid,
            &mut owner_defaulted,
        );

        let (owner, owner_sid_str) = if result.is_ok() {
            lookup_sid(owner_sid).unwrap_or_else(|_| ("Unknown".to_string(), "".to_string()))
        } else {
            ("Unknown".to_string(), "".to_string())
        };

        let mut group_sid = PSID::default();
        let mut group_defaulted = false.into();
        let result = GetSecurityDescriptorGroup(
            PSECURITY_DESCRIPTOR(buf.as_ptr() as *mut _),
            &mut group_sid,
            &mut group_defaulted,
        );

        let (group, group_sid_str) = if result.is_ok() {
            lookup_sid(group_sid)
                .unwrap_or_else(|_| ("Users".to_string(), "S-1-5-32-545".to_string()))
        } else {
            ("Users".to_string(), "S-1-5-32-545".to_string())
        };

        (owner, owner_sid_str, group, group_sid_str)
    };

    cache.insert(path_str, result.clone());

    Ok(result)
}

unsafe fn lookup_sid(sid: PSID) -> io::Result<(String, String)> {
    let mut name_len = 0u32;
    let mut domain_len = 0u32;
    let mut sid_type = SID_NAME_USE::default();

    let _ = unsafe {
        LookupAccountSidW(
            None,
            sid,
            None,
            &mut name_len,
            None,
            &mut domain_len,
            &mut sid_type,
        )
    };

    if name_len == 0 || domain_len == 0 {
        return Ok(("Unknown".to_string(), "".to_string()));
    }

    let mut name_buf = vec![0u16; name_len as usize];
    let mut domain_buf = vec![0u16; domain_len as usize];

    let result = unsafe {
        LookupAccountSidW(
            None,
            sid,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            Some(PWSTR(domain_buf.as_mut_ptr())),
            &mut domain_len,
            &mut sid_type,
        )
    };

    if result.is_err() {
        return Ok(("Unknown".to_string(), "".to_string()));
    }

    let name = if name_len > 0 {
        let name_slice = &name_buf[..name_len as usize - 1];
        String::from_utf16_lossy(name_slice)
    } else {
        String::new()
    };

    let domain = if domain_len > 0 {
        let domain_slice = &domain_buf[..domain_len as usize - 1];
        String::from_utf16_lossy(domain_slice)
    } else {
        String::new()
    };

    let sid_string = unsafe { simple_sid_to_string() };

    let display_name = if !domain.is_empty() && !name.is_empty() {
        format!("{}\\{}", domain, name)
    } else if !name.is_empty() {
        name
    } else {
        "Unknown".to_string()
    };

    Ok((display_name, sid_string))
}

unsafe fn simple_sid_to_string() -> String {
    "S-1-5-21-...".to_string()
}

pub fn calculate_inode(path: &Path) -> io::Result<u64> {
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, OPEN_EXISTING,
    };
    use windows::core::PCWSTR;

    unsafe {
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

        let handle_result = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        );

        let handle = match handle_result {
            Ok(h) => h,
            Err(_) => return Err(io::Error::last_os_error()),
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut file_info = BY_HANDLE_FILE_INFORMATION::default();
        let result = GetFileInformationByHandle(handle, &mut file_info);

        let _ = windows::Win32::Foundation::CloseHandle(handle);

        if result.is_err() {
            return Err(io::Error::last_os_error());
        }

        let inode = ((file_info.nFileIndexHigh as u64) << 32) | (file_info.nFileIndexLow as u64);
        Ok(inode)
    }
}

pub fn get_windows_permissions(
    path: &Path,
    is_symlink: bool,
    is_dir: bool,
    file_attributes: u32,
) -> String {
    let mut perms = String::with_capacity(10);

    perms.push(if is_symlink {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    });

    perms.push('r');
    perms.push(if (file_attributes & FILE_ATTRIBUTE_READONLY.0) == 0 {
        'w'
    } else {
        '-'
    });
    perms.push(if is_executable(path) { 'x' } else { '-' });

    perms.push('r');
    perms.push(if (file_attributes & FILE_ATTRIBUTE_READONLY.0) == 0 {
        'w'
    } else {
        '-'
    });
    perms.push(if is_executable(path) { 'x' } else { '-' });

    perms.push('r');
    perms.push(if (file_attributes & FILE_ATTRIBUTE_READONLY.0) == 0 {
        'w'
    } else {
        '-'
    });
    perms.push(if is_executable(path) { 'x' } else { '-' });

    let mut extra_attrs = String::new();
    if (file_attributes & FILE_ATTRIBUTE_HIDDEN.0) != 0 {
        extra_attrs.push('h');
    }
    if (file_attributes & FILE_ATTRIBUTE_SYSTEM.0) != 0 {
        extra_attrs.push('s');
    }
    if (file_attributes & FILE_ATTRIBUTE_ARCHIVE.0) != 0 {
        extra_attrs.push('a');
    }

    if !extra_attrs.is_empty() {
        perms.push(' ');
        perms.push_str(&extra_attrs);
    }

    perms
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
        } else if args.ctime && args.long {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        };

        if args.reverse { cmp.reverse() } else { cmp }
    });
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

pub fn render(entries: Vec<FileEntry>, args: &Args) -> io::Result<()> {
    if args.dired {
        print_dired(&entries);
        return Ok(());
    }

    let use_color = should_use_color(args);

    if args.one || (args.long && !args.columns && !args.across) {
        for e in &entries {
            render_entry(e, args, use_color);
        }
    } else if args.columns && !args.long && !args.across {
        render_columns(&entries, args, use_color);
    } else if args.across && !args.long {
        render_across(&entries, args, use_color);
    } else {
        render_grid(&entries, args, use_color)?;
    }
    Ok(())
}

pub fn print_dired(entries: &[FileEntry]) {
    print!("  //DIRED// ");
    for (i, _) in entries.iter().enumerate() {
        print!("{} ", i + 1);
    }
    println!("//DIRED//\n  //DIRED-OPTIONS// --dired //DIRED-OPTIONS//");
}

pub fn should_use_color(args: &Args) -> bool {
    match args.color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => std::io::stdout().is_terminal(),
    }
}

pub fn render_entry(e: &FileEntry, args: &Args, use_color: bool) {
    if args.long {
        render_long_format(e, args, use_color);
    } else if args.inode {
        println!("{:10} {}{}", e.inode, e.display_name, e.indicator);
    } else if args.size {
        let size_str = if args.human_readable {
            format_size_human(e.size)
        } else {
            format_size(e.size, &args.block_size)
        };
        println!("{:>8} {}{}", size_str, e.display_name, e.indicator);
    } else if args.author {
        println!("{} {}{}", e.owner, e.display_name, e.indicator);
    } else {
        let name_styled = style_name(e, use_color);
        println!("{}{}", name_styled, e.indicator);
    }
}

pub fn render_long_format(e: &FileEntry, args: &Args, use_color: bool) {
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
        print!("{:10} ", e.inode);
    }

    print!("{} ", e.permissions);

    print!("{:>2} ", 1);

    if args.omit_group {
        print!("{:<15} ", owner_str);
    } else {
        print!("{:<8} ", owner_str);
        if !group_str.is_empty() {
            print!("{:<8} ", group_str);
        }
    }

    if args.size || args.long {
        print!("{:>8} ", size_str);
    }

    print!("{} ", time_str);

    let name_styled = style_name(e, use_color);

    if e.is_symlink {
        match fs::read_link(&e.path) {
            Ok(target) => {
                let target_str = target.to_string_lossy();
                if use_color {
                    println!("{}{} -> {}", name_styled, e.indicator, target_str.cyan());
                } else {
                    println!("{}{} -> {}", name_styled, e.indicator, target_str);
                }
            }
            Err(_) => {
                println!("{}{} -> [broken symlink]", name_styled, e.indicator);
            }
        }
    } else {
        println!("{}{}", name_styled, e.indicator);
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

pub fn render_columns(entries: &[FileEntry], args: &Args, use_color: bool) {
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80);

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    let max_len = entries
        .iter()
        .map(|e| e.display_name.len() + e.indicator.len())
        .max()
        .unwrap_or(0)
        + 2;

    let cols = (width / max_len).max(1);
    let rows = entries.len().div_ceil(cols);

    for row in 0..rows {
        for col in 0..cols {
            let idx = col * rows + row;
            if let Some(e) = entries.get(idx) {
                let styled = style_name(e, use_color);
                print!("{:<width$}", styled, width = max_len);
            }
        }
        println!();
    }
}

pub fn render_across(entries: &[FileEntry], args: &Args, use_color: bool) {
    let term_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80);

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    let max_len = entries
        .iter()
        .map(|e| e.display_name.len() + e.indicator.len())
        .max()
        .unwrap_or(0)
        + 2;

    let cols = (width / max_len).max(1);

    for (i, e) in entries.iter().enumerate() {
        let styled = style_name(e, use_color);
        let name_len = e.display_name.len() + e.indicator.len();

        print!("{}{}", styled, e.indicator);
        if (i + 1) % cols != 0 {
            let padding = " ".repeat(max_len.saturating_sub(name_len));
            print!("{}", padding);
        }

        if (i + 1) % cols == 0 {
            println!();
        }
    }

    if !entries.len().is_multiple_of(cols) {
        println!();
    }
}

pub fn render_grid(entries: &[FileEntry], args: &Args, use_color: bool) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let term_size = terminal_size();
    let (term_width, _term_height) = match term_size {
        Some((Width(w), Height(h))) => (w as usize, h as usize),
        None => (80, 24),
    };

    let width = if args.width > 0 {
        args.width
    } else {
        term_width
    };

    let max_len = entries
        .iter()
        .map(|e| e.display_name.len() + e.indicator.len())
        .max()
        .unwrap_or(0)
        + 2;

    if max_len == 2 {
        for e in entries {
            let styled = style_name(e, use_color);
            println!("{}{}", styled, e.indicator);
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
        for col_vec in grid.iter().take(cols) {
            if let Some(e) = col_vec[row] {
                let styled = style_name(e, use_color);
                let total_width = e.display_name.len() + e.indicator.len();
                let padding = " ".repeat(max_len.saturating_sub(total_width));
                print!("{}{}{}", styled, e.indicator, padding);
            }
        }
        println!();
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

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
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

    if s.chars().all(|c| c.is_alphabetic()) {
        return match s.as_str() {
            "K" | "KB" => 1024,
            "M" | "MB" => 1024 * 1024,
            "G" | "GB" => 1024 * 1024 * 1024,
            "T" | "TB" => 1024 * 1024 * 1024 * 1024,
            _ => 1,
        };
    }

    let num_part: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let suffix: String = s.chars().skip_while(|c| c.is_ascii_digit()).collect();

    let base: u64 = if num_part.is_empty() {
        1
    } else {
        num_part.parse().unwrap_or(1)
    };

    let mult = match suffix.as_str() {
        "K" | "KB" => 1024,
        "M" | "MB" => 1024 * 1024,
        "G" | "GB" => 1024 * 1024 * 1024,
        "T" | "TB" => 1024 * 1024 * 1024 * 1024,
        _ => 1,
    };

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
