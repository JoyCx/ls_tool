pub fn is_executable(path: &std::path::Path) -> bool {
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

/// Cache utility for retrieving or computing values with fallible computation.
///
/// This function checks if a value exists in the cache. If it does, returns it.
/// If not, computes it using the provided closure and stores it in the cache.
///
/// # Arguments
/// * `cache` - An OnceLock wrapping a Mutex around a HashMap
/// * `key` - The cache key (must be Hash + Eq)
/// * `compute` - Closure that returns `io::Result<T>` if cache miss
///
/// # Example
/// ```ignore
/// let result = cache_get_or_compute(&MY_CACHE, path_string, || {
///     expensive_operation()
/// })?;
/// ```
pub fn cache_get_or_compute<T, K, F>(
    cache: &std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<K, T>>>,
    key: K,
    compute: F,
) -> std::io::Result<T>
where
    T: Clone,
    K: Clone + std::hash::Hash + Eq,
    F: FnOnce() -> std::io::Result<T>,
{
    let cache_mutex = cache.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));

    // Try to get from cache
    if let Ok(cache_lock) = cache_mutex.lock()
        && let Some(cached) = cache_lock.get(&key)
    {
        return Ok(cached.clone());
    }

    // Compute the value
    let result = compute()?;

    // Store in cache
    if let Ok(mut cache_lock) = cache_mutex.lock() {
        cache_lock.insert(key, result.clone());
    }

    Ok(result)
}

/// Cache utility for retrieving or computing values with infallible computation.
///
/// This function checks if a value exists in the cache. If it does, returns it.
/// If not, computes it using the provided closure and stores it in the cache.
///
/// # Arguments
/// * `cache` - An OnceLock wrapping a Mutex around a HashMap
/// * `key` - The cache key (must be Hash + Eq)
/// * `compute` - Closure that returns `T` (always succeeds)
///
/// # Example
/// ```ignore
/// let result = cache_get_or_compute_sync(&MY_CACHE, cache_key, || {
///     format_string(attrs)
/// });
/// ```
pub fn cache_get_or_compute_sync<T, K, F>(
    cache: &std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<K, T>>>,
    key: K,
    compute: F,
) -> T
where
    T: Clone,
    K: Clone + std::hash::Hash + Eq,
    F: FnOnce() -> T,
{
    let cache_mutex = cache.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));

    // Try to get from cache
    if let Ok(cache_lock) = cache_mutex.lock()
        && let Some(cached) = cache_lock.get(&key)
    {
        return cached.clone();
    }

    // Compute the value
    let result = compute();

    // Store in cache
    if let Ok(mut cache_lock) = cache_mutex.lock() {
        cache_lock.insert(key, result.clone());
    }

    result
}
