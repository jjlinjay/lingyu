//! 外部通知桥：OpenCode 会话结束后通过本地 HTTP 推送到岛内气泡。
//!
//! 插件由本模块启动时写入 `~/.config/opencode/plugins/lingyu/index.mjs`，
//! 灵屿监听 127.0.0.1:23334 起的一组端口，收到 `{type,title,session_id}` 后
//! 交给 App 显示「哪个会话完成了」。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 会话完成，附带会话标题（可能为空）。
    Done(String),
    /// 会话失败，附带会话标题（可能为空）。
    Failed(String),
}

pub struct Notifier {
    rx: Receiver<Event>,
    pub port: u16,
}

const PLUGIN_JS: &str = r#"// 灵屿通知桥：OpenCode 会话完成/失败时提醒到岛内。
import { appendFileSync } from "node:fs";
import { join } from "node:path";

const ENDPOINT = "http://127.0.0.1:23334/opencode";
const LOG = join(process.env.TEMP || ".", "lingyu-events.log");
const titles = new Map();

function log(text) {
  try { appendFileSync(LOG, `${new Date().toISOString()} ${text}\n`); } catch {}
}

function cleanTitle(value) {
  if (typeof value !== "string") return "";
  return value.replace(/[\u0000-\u001f\u007f]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 80);
}

function sessionInfo(envelope) {
  const props = envelope && envelope.properties && typeof envelope.properties === "object" ? envelope.properties : {};
  const data = envelope && envelope.data && typeof envelope.data === "object" ? envelope.data : {};
  const info = props.info && typeof props.info === "object" && !Array.isArray(props.info) ? props.info : {};
  const loc = envelope && envelope.location && typeof envelope.location === "object" ? envelope.location : {};
  const id = data.sessionID || props.sessionID || envelope.sessionID || info.id || "";
  let title = cleanTitle(info.title);
  if (id && title) titles.set(id, title);
  if (!title && id) title = titles.get(id) || "";
  return { id, title, directory: loc.directory || info.directory || "" };
}

async function post(type, envelope) {
  const meta = sessionInfo(envelope);
  log(`post ${type} title=${meta.title}`);
  try {
    await fetch(ENDPOINT, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ type, title: meta.title, session_id: meta.id, directory: meta.directory }),
    });
  } catch (err) { log(`post failed ${err && err.message}`); }
}

export default {
  id: "lingyu-notify",
  async setup(ctx) {
    log("setup");
    const controller = new AbortController();
    (async () => {
      for await (const envelope of ctx.event.subscribe({ signal: controller.signal })) {
        const type = envelope && envelope.type;
        if (!type) continue;
        if (type === "session.created" || type === "session.updated") {
          const meta = sessionInfo(envelope);
          if (meta.id && meta.title) titles.set(meta.id, meta.title);
          continue;
        }
        if (/execution|error|idle/.test(type)) log(`event ${type}`);
        if (type === "session.execution.succeeded") {
          void post("done", envelope);
        } else if (type === "session.execution.failed" || type === "session.error") {
          void post("failed", envelope);
        }
      }
    })().catch((err) => log(`stream end ${err && err.message}`));
    return () => { controller.abort(); };
  },
};
"#;

impl Notifier {
    pub fn start() -> Self {
        let (tx, rx) = channel();
        let port = serve(tx);
        if port > 0 {
            install_plugin(port);
        }
        Self { rx, port }
    }

    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }
}

fn serve(tx: Sender<Event>) -> u16 {
    let mut listener = None;
    for port in 23334..=23444u16 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => {
                listener = Some((l, port));
                break;
            }
            Err(_) => continue,
        }
    }
    let Some((listener, port)) = listener else {
        log_line("no free port");
        return 0;
    };
    log_line(&format!("listening on {port}"));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
            handle(stream, &tx);
        }
    });
    port
}

fn handle(mut stream: TcpStream, tx: &Sender<Event>) {
    let mut data = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&data);
                if let Some(split) = text.find("\r\n\r\n") {
                    let header = &text[..split];
                    let content_length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if data.len() >= split + 4 + content_length {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&data);
    if text.starts_with("GET ") {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");
        return;
    }
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    let head = text[..text.len().min(120)].replace(['\r', '\n'], " ");
    log_line(&format!("req {head} body={body}"));
    let value = serde_json::from_str::<serde_json::Value>(body).unwrap_or(serde_json::Value::Null);
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or_default();
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let session_id = value
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = if title.is_empty() && !session_id.is_empty() {
        session_title(&session_id).unwrap_or_default()
    } else {
        title
    };
    let event = match kind {
        "done" => Some(Event::Done(title)),
        "failed" => Some(Event::Failed(title)),
        _ => None,
    };
    if let Some(ev) = event {
        let _ = tx.send(ev);
    }
    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\n\r\n");
}

/// 从 OpenCode 本地数据库按会话 id 取标题（插件事件里不带标题）。
fn session_title(session_id: &str) -> Option<String> {
    let path = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)?
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db");
    if !path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    // v2 把会话存在 session_v2，旧库仍在 session；两边都试一次。
    for table in ["session_v2", "session"] {
        let sql = format!("select title from {table} where id = ?1");
        let found = conn
            .query_row(&sql, [session_id], |row| row.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .filter(|t| !t.trim().is_empty());
        if found.is_some() {
            return found;
        }
    }
    None
}

fn install_plugin(port: u16) {
    let Some(home) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from) else {
        return;
    };
    let dir = home.join(".config").join("opencode").join("plugins");
    if !dir.parent().map(|p| p.is_dir()).unwrap_or(false) {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let dir = dir.join("lingyu");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("index.mjs");
    let body = PLUGIN_JS.replace("23334", &port.to_string());
    let stale = std::fs::read_to_string(&path)
        .map(|old| old != body)
        .unwrap_or(true);
    if stale {
        let _ = std::fs::write(path, body);
    }
}

fn log_line(text: &str) {
    let path = std::env::temp_dir().join("lingyu-notify.log");
    let line = format!(
        "{} {}\n",
        chrono::Local::now().format("%H:%M:%S"),
        text
    );
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}
