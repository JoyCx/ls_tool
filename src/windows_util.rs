use crate::{cache_get_or_compute, cache_get_or_compute_sync, is_executable};
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

type SidCacheMap = HashMap<String, (String, String, String, String)>;
type FileAttributesCacheMap = HashMap<String, u32>;
type InodeCacheMap = HashMap<String, u64>;
type PermissionsCacheMap = HashMap<(String, u32), String>;

// Cache for file ownership/group (SID lookups)
// Mutex ensures thread safety during cache operations
static SID_CACHE: OnceLock<Mutex<SidCacheMap>> = OnceLock::new();

// Cache for file attributes by path
// Prevents repeated GetFileAttributesW calls for the same file
static FILE_ATTRIBUTES_CACHE: OnceLock<Mutex<FileAttributesCacheMap>> = OnceLock::new();

// Cache for inode calculations by path
// Prevents repeated file_id lookups and hash computations
static INODE_CACHE: OnceLock<Mutex<InodeCacheMap>> = OnceLock::new();

// Cache for permission strings (keyed by path + file_attributes)
// Prevents recalculating the same permission string multiple times
static PERMISSIONS_CACHE: OnceLock<Mutex<PermissionsCacheMap>> = OnceLock::new();

pub fn get_file_attributes_windows(path: &Path) -> io::Result<u32> {
    use std::os::windows::fs::MetadataExt;

    let path_str = path.to_string_lossy().to_string();

    cache_get_or_compute(&FILE_ATTRIBUTES_CACHE, path_str, || {
        let metadata = fs::metadata(path)?;
        Ok(metadata.file_attributes())
    })
}

pub fn get_owner_and_group(path: &Path) -> io::Result<(String, String, String, String)> {
    let path_str = path.to_string_lossy().to_string();

    cache_get_or_compute(&SID_CACHE, path_str, || {
        // Get the security descriptor
        let sd = GetNamedSecurityInfo(
            path,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner | SecurityInformation::Group,
        )
        .map_err(|e| io::Error::other(format!("Failed to get security info: {}", e)))?;

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

        Ok((owner_name, String::new(), group_name, String::new()))
    })
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
    let path_str = path.to_string_lossy().to_string();

    cache_get_or_compute(&INODE_CACHE, path_str, || {
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
            .map_err(std::io::Error::other)
    })
}

pub fn get_windows_permissions(
    path: &Path,
    is_symlink: bool,
    is_dir: bool,
    file_attributes: u32,
) -> String {
    let path_str = path.to_string_lossy().to_string();
    let cache_key = (path_str, file_attributes);

    cache_get_or_compute_sync(&PERMISSIONS_CACHE, cache_key, || {
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
    })
}
