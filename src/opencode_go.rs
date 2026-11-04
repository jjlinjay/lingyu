//! OpenCode Go 额度：复用本机 OpenCode 已登录的 workspace key。
use super::{Snapshot, Window};
use serde_json::Value;
use rusqlite::{Connection, OpenFlags};
use std::{path::{Path, PathBuf}};

pub const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

pub fn db_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let path = PathBuf::from(home).join(".local/share/opencode/opencode.db");
    path.is_file().then_some(path)
}

fn open(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
        .map_err(|_| "无法读取 OpenCode 登录数据库".to_string())
}

fn key_of(value: &str) -> Option<String> {
    let root: Value = serde_json::from_str(value).ok()?;
    root.get("key").and_then(Value::as_str).filter(|key| !key.trim().is_empty()).map(str::to_owned)
}

pub struct Entry { pub id: String }

pub fn entries(path: &Path) -> Result<Vec<Entry>, String> {
    let connection = open(path)?;
    let mut statement = connection
        .prepare("SELECT id, value FROM credential WHERE integration_id IN ('opencode-go','opencode') ORDER BY active DESC, time_updated DESC")
        .map_err(|_| "OpenCode 登录数据库结构不兼容".to_string())?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|_| "无法读取 OpenCode 登录信息".to_string())?;
    let mut found = Vec::new();
    for row in rows.flatten() {
        if key_of(&row.1).is_some() { found.push(Entry { id: row.0 }); }
    }
    if found.is_empty() { return Err("OpenCode 里还没有登录 OpenCode Go".into()); }
    Ok(found)
}

pub fn token(path: &Path, slot: &str) -> Result<String, String> {
    let connection = open(path)?;
    let value: String = connection
        .query_row("SELECT value FROM credential WHERE id = ?1", [slot], |row| row.get(0))
        .map_err(|_| "OpenCode 登录信息已失效，请重新登录".to_string())?;
    key_of(&value).ok_or_else(|| "OpenCode 登录信息已失效，请重新登录".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_windows_are_mapped_to_remaining_percent() {
        let payload = json!({"usage":{
            "rolling":{"status":"ok","percent":37,"resetsAt":"2030-01-01T12:00:00Z"},
            "weekly":{"status":"rate-limited","percent":10,"resetsAt":"2030-01-05T12:00:00Z"},
            "monthly":{"status":"rate-limited","percent":10,"resetsAt":"2030-02-01T00:00:00Z"}
        }});
        let snapshot = parse(&payload).unwrap();
        assert_eq!(snapshot.plan, "OpenCode Go");
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].label, "5 小时");
        assert!((snapshot.windows[0].remaining - 63.0).abs() < 0.01);
        assert_eq!(snapshot.windows[1].label, "本月");
        assert!((snapshot.windows[1].remaining - 0.0).abs() < 0.01, "rate-limited 视为已用满");
        assert!(snapshot.windows[0].reset.is_some());
    }

    #[test]
    fn provider_name_and_alias_parse() {
        assert_eq!(super::super::Provider::parse("OpenCode Go"), Some(super::super::Provider::OpenCodeGo));
        assert_eq!(super::super::Provider::parse("opencode-go"), Some(super::super::Provider::OpenCodeGo));
    }
}

pub fn parse(v: &Value) -> Result<Snapshot, String> {
    let usage = &v["usage"];
    if !usage.is_object() { return Err("服务没有返回额度信息".into()); }
    let mut windows = Vec::new();
    for (key, label, seconds) in [("rolling", "5 小时", 18000i64), ("monthly", "本月", 2592000)] {
        let block = &usage[key];
        let percent = block["percent"].as_f64();
        let limited = block["status"].as_str() == Some("rate-limited");
        let used = if limited { Some(100.0) } else { percent };
        let Some(used) = used else { continue; };
        let reset = block["resetsAt"].as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp());
        windows.push(Window { label: label.into(), seconds: Some(seconds), remaining: (100.0 - used).clamp(0.0, 100.0) as f32, reset });
    }
    if windows.is_empty() { return Err("服务未提供周期额度".into()); }
    Ok(Snapshot { plan: "OpenCode Go".into(), windows, extra: vec![], at: chrono::Utc::now().timestamp() })
}
