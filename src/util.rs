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
