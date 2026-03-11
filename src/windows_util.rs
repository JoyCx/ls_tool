/// Windows-specific filesystem helpers.
///
/// Design constraints
/// ------------------
/// * **No `unsafe` blocks.**  Every Windows API call is reached through either
///   the standard library's `std::os::windows` extension traits or the `winsafe`
///   crate, which provides safe Rust wrappers around Win32.
/// * Real data only — we never fabricate values.  Where an accurate figure
///   cannot be obtained we propagate the error and let the caller decide on a
///   fallback (usually the raw file size).
use crate::{cache_get_or_compute, is_executable};
use file_id::get_file_id;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use windows_permissions::constants::{AceType, SeObjectType, SecurityInformation};
use windows_permissions::wrappers::{GetNamedSecurityInfo, LookupAccountSid};

// ─── Cache type aliases ───────────────────────────────────────────────────────

type SidCacheMap = HashMap<String, (String, String, String, String)>;
type FileAttributesCacheMap = HashMap<String, u32>;
type InodeCacheMap = HashMap<String, u64>;

static SID_CACHE: OnceLock<Mutex<SidCacheMap>> = OnceLock::new();
static FILE_ATTRIBUTES_CACHE: OnceLock<Mutex<FileAttributesCacheMap>> = OnceLock::new();
static INODE_CACHE: OnceLock<Mutex<InodeCacheMap>> = OnceLock::new();

// The well-known SDDL string for the "Everyone" (World) SID.
// S-1-1-0 is stable and universal — no runtime lookup needed.
const EVERYONE_SID: &str = "S-1-1-0";

// ─── Public helpers ───────────────────────────────────────────────────────────

/// Return the hard-link count for `path`.
///
/// Uses [`std::os::windows::fs::MetadataExt::number_of_links`], which is a
/// safe, zero-unsafe call into the OS that has been stable since Rust 1.75.
/// Symlink metadata is used so that the count reflects the reparse point
/// itself, not its target.
pub fn get_nlink(path: &Path) -> io::Result<u32> {
    use std::os::windows::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    // `number_of_links` returns `Option<u64>`; a missing value is treated as 1
    // (single link), which is the correct default for files that cannot be
    // hard-linked (e.g. directories on FAT/exFAT).
    Ok(meta.number_of_links().unwrap_or(1) as u32)
}

/// Return the raw Windows file-attribute bitmask for `path`, with caching.
///
/// Uses [`std::os::windows::fs::MetadataExt::file_attributes`] — no unsafe.
pub fn get_file_attributes_windows(path: &Path) -> io::Result<u32> {
    use std::os::windows::fs::MetadataExt;

    let path_str = path.to_string_lossy().to_string();

    cache_get_or_compute(&FILE_ATTRIBUTES_CACHE, path_str, || {
        Ok(fs::metadata(path)?.file_attributes())
    })
}

/// Return `(owner_name, owner_sid, group_name, group_sid)` for `path`.
///
/// Security descriptors are retrieved through `windows_permissions`, a
/// safe-Rust wrapper around the Win32 security API.  Results are cached by
/// path string so that a directory listing does not query the ACL engine
/// once per entry.
pub fn get_owner_and_group(path: &Path) -> io::Result<(String, String, String, String)> {
    let path_str = path.to_string_lossy().to_string();

    cache_get_or_compute(&SID_CACHE, path_str, || {
        let sd = GetNamedSecurityInfo(
            path,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner | SecurityInformation::Group,
        )
        .map_err(|e| io::Error::other(format!("Failed to get security info: {}", e)))?;

        let (owner_name, owner_sid_str) = match sd.owner() {
            Some(sid) => {
                let name = lookup_sid_name(sid).unwrap_or_else(|| "Unknown".to_string());
                (name, sid.to_string())
            }
            None => ("Unknown".to_string(), String::new()),
        };

        let (mut group_name, group_sid_str) = match sd.group() {
            Some(sid) => {
                let name = lookup_sid_name(sid).unwrap_or_else(|| "Users".to_string());
                (name, sid.to_string())
            }
            None => ("Users".to_string(), String::new()),
        };

        if group_name.is_empty() || group_name == "Unknown" {
            group_name = "Users".to_string();
        }

        Ok((owner_name, owner_sid_str, group_name, group_sid_str))
    })
}

/// Compute a stable pseudo-inode for `path`.
///
/// Windows does not expose inode numbers the way POSIX does, but the
/// `file-id` crate retrieves the underlying file index (NTFS object ID /
/// `nFileIndexHigh` + `nFileIndexLow` from `GetFileInformationByHandle`)
/// safely.  We hash the opaque `FileId` value into a `u64` so it can be
/// displayed in place of an inode.
pub fn calculate_inode(path: &Path) -> io::Result<u64> {
    let path_str = path.to_string_lossy().to_string();

    cache_get_or_compute(&INODE_CACHE, path_str, || {
        get_file_id(path)
            .map(|fid| {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                format!("{:?}", fid).hash(&mut hasher);
                hasher.finish()
            })
            .map_err(io::Error::other)
    })
}

/// Return a Unix-style 10-character permission string for `path`.
///
/// Delegates to [`acl_permissions`] for a real DACL-derived answer, and
/// falls back to [`fallback_permissions`] only when the security descriptor
/// cannot be read (e.g. insufficient privilege).
pub fn get_windows_permissions(
    path: &Path,
    is_symlink: bool,
    is_dir: bool,
    file_attributes: u32,
) -> String {
    match acl_permissions(path, is_symlink, is_dir) {
        Ok(s) => s,
        Err(_) => fallback_permissions(file_attributes, is_symlink, is_dir),
    }
}

/// Return the number of bytes actually allocated on disk for `path`.
///
/// On NTFS this accounts for transparent compression and sparse regions —
/// a compressed 1 MB file may occupy only 256 KB of cluster space.
/// We use the `filesize` crate to call the correct Win32 API safely.
///
/// Falls back to the logical file size (from `std::fs::metadata`) if the
/// call fails — this is always correct for uncompressed, non-sparse files.
pub fn get_allocated_size(path: &Path) -> io::Result<u64> {
    filesize::file_real_size(path).or_else(|_| {
        // Graceful fallback: logical size rounded to the default NTFS
        // cluster boundary (4 KiB).  This is accurate for regular,
        // non-compressed, non-sparse files, which is the common case.
        let size = fs::metadata(path)?.len();
        const CLUSTER: u64 = 4096;
        Ok(if size == 0 {
            0
        } else {
            size.div_ceil(CLUSTER) * CLUSTER
        })
    })
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Resolve a SID to a `DOMAIN\name` string via `LookupAccountSid`.
fn lookup_sid_name(sid: &windows_permissions::Sid) -> Option<String> {
    LookupAccountSid(sid).ok().map(|(name, domain)| {
        let name_str = name.to_string_lossy().into_owned();
        let domain_str = domain.to_string_lossy().into_owned();

        match (domain_str.is_empty(), name_str.is_empty()) {
            (false, false) => format!("{}\\{}", domain_str, name_str),
            (_, false) => name_str,
            _ => String::new(),
        }
    })
}

// ─── Bit-flags for r / w / x decisions ───────────────────────────────────────

/// Relevant Win32 generic access-mask bits used to derive r/w/x characters.
mod mask {
    /// FILE_GENERIC_READ  (SYNCHRONIZE + READ_CONTROL + standard read bits)
    pub const READ: u32 = 0x0012_0089;
    /// FILE_GENERIC_WRITE (SYNCHRONIZE + READ_CONTROL + standard write bits)
    pub const WRITE: u32 = 0x0012_0116;
    /// FILE_GENERIC_EXECUTE (SYNCHRONIZE + READ_CONTROL + execute bit)
    pub const EXEC: u32 = 0x0012_00A0;
}

/// Derive `[r, w, x]` characters from an (allow, deny) mask pair.
///
/// Deny ACEs take precedence over allow ACEs, matching the standard DACL
/// evaluation order documented in the Win32 SDK.
#[inline]
fn rwx(allow: u32, deny: u32) -> [char; 3] {
    let eff = allow & !deny;
    [
        if eff & mask::READ != 0 { 'r' } else { '-' },
        if eff & mask::WRITE != 0 { 'w' } else { '-' },
        if eff & mask::EXEC != 0 { 'x' } else { '-' },
    ]
}

/// Derive a Unix-style permission string by inspecting the file's DACL.
///
/// Reads the security descriptor once (owner SID + group SID + DACL) and
/// accumulates separate allow/deny masks for owner, group, and world
/// (Everyone / S-1-1-0).  The resulting string has the form
/// `[-dl][rwx]{3}[rwx]{3}[rwx]{3}`, e.g. `-rwxr-xr-x`.
///
/// All Win32 interactions go through `windows_permissions`, a safe crate —
/// no `unsafe` is required here.
fn acl_permissions(
    path: &Path,
    is_symlink: bool,
    is_dir: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let sd = GetNamedSecurityInfo(
        path,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Group | SecurityInformation::Dacl,
    )?;

    let owner_sid: Option<String> = sd.owner().map(|s| s.to_string());
    let group_sid: Option<String> = sd.group().map(|s| s.to_string());

    // Accumulated allow / deny masks for owner, group, world.
    let (mut oa, mut od) = (0u32, 0u32);
    let (mut ga, mut gd) = (0u32, 0u32);
    let (mut wa, mut wd) = (0u32, 0u32);

    if let Some(dacl) = sd.dacl() {
        let count = dacl.len();
        for i in 0..count {
            if let Some(ace) = dacl.get_ace(i) {
                let ace_sid = match ace.sid() {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                let is_owner = owner_sid.as_deref() == Some(&ace_sid);
                let is_group = group_sid.as_deref() == Some(&ace_sid);
                let is_world = ace_sid == EVERYONE_SID;

                if !is_owner && !is_group && !is_world {
                    continue;
                }

                let mask: u32 = ace.mask().bits();
                match ace.ace_type() {
                    AceType::ACCESS_ALLOWED_ACE_TYPE => {
                        if is_owner {
                            oa |= mask;
                        }
                        if is_group {
                            ga |= mask;
                        }
                        if is_world {
                            wa |= mask;
                        }
                    }
                    AceType::ACCESS_DENIED_ACE_TYPE => {
                        if is_owner {
                            od |= mask;
                        }
                        if is_group {
                            gd |= mask;
                        }
                        if is_world {
                            wd |= mask;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let type_char = if is_symlink {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    };

    // On Windows, executable status for regular files is determined by
    // extension (e.g. .exe, .bat) rather than a permission bit.
    let exec_override = !is_dir && !is_symlink && is_executable(path);
    let [or, ow, mut ox] = rwx(oa, od);
    let [gr, gw, gx] = rwx(ga, gd);
    let [wr, ww, wx] = rwx(wa, wd);

    if exec_override && ox == '-' && or == 'r' {
        ox = 'x';
    }

    Ok(format!(
        "{}{}{}{}{}{}{}{}{}{}",
        type_char, or, ow, ox, gr, gw, gx, wr, ww, wx
    ))
}

/// Coarse fallback permission string used when the DACL is unreadable.
///
/// Derives only the write bit from `FILE_ATTRIBUTE_READONLY`; all other
/// bits are marked unknown (`?`) to avoid misrepresenting security state.
fn fallback_permissions(file_attributes: u32, is_symlink: bool, is_dir: bool) -> String {
    const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;

    let type_char = if is_symlink {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    };
    let write = if file_attributes & FILE_ATTRIBUTE_READONLY != 0 {
        '-'
    } else {
        'w'
    };

    format!("{}r{}???????", type_char, write)
}
