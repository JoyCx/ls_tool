use crate::is_executable;
use file_id::get_file_id;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_SYSTEM, GetFileAttributesW, GetFileInformationByHandle,
};

use windows_permissions::constants::{SeObjectType, SecurityInformation};
use windows_permissions::wrappers::{GetNamedSecurityInfo, LookupAccountSid};

static SID_CACHE: OnceLock<Mutex<HashMap<String, (String, String, String, String)>>> =
    OnceLock::new();

pub fn get_file_attributes_windows(path: &Path) -> io::Result<u32> {
    use std::os::windows::fs::MetadataExt;

    let metadata = fs::metadata(path)?;
    Ok(metadata.file_attributes())
}

pub fn get_owner_and_group(path: &Path) -> io::Result<(String, String, String, String)> {
    let cache_mutex = SID_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let path_str = path.to_string_lossy().to_string();

    // Check cache
    if let Ok(cache) = cache_mutex.lock() {
        if let Some(cached) = cache.get(&path_str) {
            return Ok(cached.clone());
        }
    }

    // Get the security descriptor
    let sd = GetNamedSecurityInfo(
        path,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Group,
    )
    .map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Failed to get security info: {}", e),
        )
    })?;

    // Extract owner SID
    let owner_name = if let Some(owner_sid) = sd.owner() {
        lookup_sid_name(owner_sid).unwrap_or_else(|| "Unknown".to_string())
    } else {
        "Unknown".to_string()
    };

    // Extract group SID
    let mut group_name = if let Some(group_sid) = sd.group() {
        lookup_sid_name(group_sid).unwrap_or_else(|| "Users".to_string())
    } else {
        "Users".to_string()
    };

    // Default fallback for group
    if group_name == "Unknown" || group_name.is_empty() {
        group_name = "Users".to_string();
    }

    let result = (owner_name, String::new(), group_name, String::new());

    // Update cache
    if let Ok(mut cache) = cache_mutex.lock() {
        cache.insert(path_str.clone(), result.clone());
    }

    Ok(result)
}

fn lookup_sid_name(sid: &windows_permissions::Sid) -> Option<String> {
    LookupAccountSid(sid).ok().map(|(name, domain)| {
        let name_str = name.to_string_lossy().into_owned();
        let domain_str = domain.to_string_lossy().into_owned();

        if !domain_str.is_empty() && !name_str.is_empty() {
            let mut result = domain_str;
            result.push('\\');
            result.push_str(&name_str);
            result
        } else if !name_str.is_empty() {
            name_str
        } else {
            String::new()
        }
    })
}

pub fn calculate_inode(path: &Path) -> std::io::Result<u64> {
    get_file_id(path)
        .map(|fid| {
            // get_file_id returns a FileId that contains the inode info
            // Hash it to get a u64 representation
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            format!("{:?}", fid).hash(&mut hasher);
            hasher.finish()
        })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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
