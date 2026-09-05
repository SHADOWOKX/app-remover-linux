use cleanly_core::Result;
use std::path::Path;
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    pub name: String,
    pub icon: String,
    pub exec: String,
    pub flatpak: Option<String>,
    pub snap: Option<String>,
}
/// Parses only bounded metadata in [Desktop Entry]. Never evaluates Exec, expansions or escapes.
pub fn parse(text: &str) -> Result<DesktopEntry> {
    if text.len() > 128 * 1024 {
        return Err("Desktop entry too large".into());
    }
    let mut section = false;
    let mut fields = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line == "[Desktop Entry]";
            continue;
        }
        if !section || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && fields.insert(key, value).is_some()
        {
            return Err("Duplicate desktop key".into());
        }
    }
    if fields.get("Type") != Some(&"Application")
        || fields.get("Hidden") == Some(&"true")
        || fields.get("NoDisplay") == Some(&"true")
    {
        return Err("Not a visible graphical application".into());
    }
    let name = fields.get("Name").ok_or("Missing application name")?;
    if name.chars().any(char::is_control) {
        return Err("Invalid display name".into());
    }
    Ok(DesktopEntry {
        name: (*name).into(),
        icon: fields
            .get("Icon")
            .unwrap_or(&"application-x-executable-symbolic")
            .to_string(),
        exec: fields.get("Exec").unwrap_or(&"").to_string(),
        flatpak: fields.get("X-Flatpak").map(|s| s.to_string()),
        snap: fields.get("X-SnapInstanceName").map(|s| s.to_string()),
    })
}
pub fn read(path: &Path) -> Result<DesktopEntry> {
    parse(&crate::files::read_regular(path, 128 * 1024)?)
}
/// Restricted exact absolute executable only. Ambiguous quoting/field codes are kept unassociated.
pub fn exact_executable(exec: &str) -> Option<String> {
    let first = if let Some(rest) = exec.strip_prefix('"') {
        rest.split_once('"')?.0
    } else {
        exec.split_whitespace().next()?
    };
    if !first.starts_with('/')
        || first
            .chars()
            .any(|c| matches!(c, '\\' | '%' | '$' | '`' | '\n' | '\r'))
    {
        None
    } else {
        Some(first.into())
    }
}
