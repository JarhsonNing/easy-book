//! Read only the input and assistant text from app-owned task logs.
use super::*;
use std::io::Read;

const LOG_LIMIT: u64 = 2 * 1024 * 1024;
const RUN_LIMIT: usize = 20;

fn read_text(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(LOG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LOG_LIMIT {
        return Err("对话记录过大，无法显示".into());
    }
    // A process can be in the middle of writing a UTF-8 character.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn log_info(path: &Path) -> Value {
    match fs::metadata(path) {
        Ok(meta) => json!({"bytes":meta.len(),"updatedAt":meta.modified().ok().map(|t|chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())}),
        Err(_) => Value::Null,
    }
}

fn reply(output: &str, engine: &str) -> String {
    let mut parts = Vec::new();
    let events = if let Ok(event) = serde_json::from_str::<Value>(output) {
        vec![event]
    } else {
        output
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect()
    };
    for event in events {
        if engine == "qoder" {
            if event["type"] == "result"
                && event["subtype"] == "success"
                && event["is_error"] != true
            {
                if let Some(text) = event["result"].as_str() {
                    return text.to_owned();
                }
            }
            if event["type"] == "assistant" {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "text" {
                        if let Some(text) = block["text"].as_str() {
                            parts.push(text.to_owned());
                        }
                    }
                }
            }
        } else if event["type"] == "text" {
            if let Some(text) = event["part"]["text"]
                .as_str()
                .or_else(|| event["text"].as_str())
            {
                parts.push(text.to_owned());
            }
        }
    }
    parts.concat()
}

pub(crate) fn read(s: &AppState, job: &Value, bid: i64) -> Result<Value, String> {
    if job["kind"] != "ai"
        || !job["result"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| i["bookId"] == bid))
    {
        return Err("图书不属于此 AI 任务".into());
    }
    let root = support(s).join("workspace/ai-jobs");
    let prefix = format!("{}-{bid}-", job["id"].as_i64().ok_or("任务编号无效")?);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("无法读取对话记录：{e}")),
    };
    let mut dirs = Vec::new();
    for entry in entries.into_iter().flatten() {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name
            .strip_prefix(&prefix)
            .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .map_err(|e| e.to_string())?;
        dirs.push((modified, name, entry.path()));
    }
    dirs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let truncated = dirs.len() > RUN_LIMIT;
    dirs.truncate(RUN_LIMIT);
    dirs.reverse();
    let engine = job["request"]["provider"].as_str().unwrap_or("opencode");
    let mut runs = Vec::new();
    for (modified, name, dir) in dirs {
        let meta = read_text(&dir.join("conversation.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or(json!({}));
        let input = read_text(&dir.join("input.txt"));
        let output = if dir.join("stdout.ndjson").exists() {
            read_text(&dir.join("stdout.ndjson"))
        } else {
            Ok(String::new())
        };
        let mut error = input
            .as_ref()
            .err()
            .map(|_| "无法读取本次输入资料".to_string())
            .unwrap_or_default();
        if let Err(e) = &output {
            error = e.clone();
        }
        let system = meta["system"].as_str().map(str::to_owned).or_else(|| {
            read_text(&dir.join("opencode.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| {
                    v["agents"]["taoshu-writer"]["system"]
                        .as_str()
                        .map(str::to_owned)
                })
        });
        let created = meta["createdAt"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339());
        let execution = read_text(&dir.join("execution.json")).ok()
            .and_then(|raw|serde_json::from_str::<Value>(&raw).ok()).unwrap_or(Value::Null);
        runs.push(json!({"id":name,"createdAt":created,"prompt":meta["prompt"].as_str().or_else(||job["request"]["prompt"].as_str()),
            "execution":execution,"stdout":log_info(&dir.join("stdout.ndjson")),"stderr":log_info(&dir.join("stderr.log")),
            "system":system,"input":input.unwrap_or_default(),"reply":reply(&output.unwrap_or_default(),engine),"error":error}));
    }
    Ok(json!({"bookId":bid,"runs":runs,"truncated":truncated}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_assistant_text_and_tolerates_incomplete_events() {
        let output = "{\"type\":\"reasoning\",\"part\":{\"text\":\"private reasoning\"}}\n{\"type\":\"text\",\"part\":{\"text\":\"正文\"}}\n{\"type\":\"error\",\"message\":\"provider-secret\"}\n{\"type\":\"text\"";
        assert_eq!(reply(output, "opencode"), "正文");
        let output = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"thinking\",\"text\":\"private\"},{\"type\":\"text\",\"text\":\"部分正文\"}]}}\n{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"最终正文\"}";
        assert_eq!(reply(output, "qoder"), "最终正文");
        assert_eq!(reply("{\n  \"type\": \"result\",\n  \"subtype\": \"success\",\n  \"result\": \"多行格式回复\"\n}","qoder"),"多行格式回复");
        assert_eq!(reply("{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true,\"result\":\"provider-secret\"}","qoder"),"");
    }

    #[test]
    fn task_logs_are_scoped_and_preserve_attempts_and_old_records() {
        let path =
            std::env::temp_dir().join(format!("taoshu-conversation-{}", rand::random::<u64>()));
        let s = AppState::new(path.clone(), 0).unwrap();
        let job = json!({"id":12,"kind":"ai","request":{"prompt":"原始提示词"},"result":{"items":[{"bookId":3}]}});
        let root = support(&s).join("workspace/ai-jobs");
        for name in ["12-3-1", "12-3-2", "12-4-1", "123-3-1"] {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("input.txt"), name).unwrap();
            fs::write(
                dir.join("stdout.ndjson"),
                "{\"type\":\"text\",\"text\":\"已生成\"}\n",
            )
            .unwrap();
            fs::write(dir.join("stderr.log"), "private-provider-error").unwrap();
        }
        fs::write(
            root.join("12-3-2/conversation.json"),
            json!({"prompt":"本次提示词","system":"系统规则","createdAt":"2026-10-29T00:00:00Z"})
                .to_string(),
        )
        .unwrap();
        fs::write(root.join("12-3-2/execution.json"),json!({"pid":42,"status":"exited","exitCode":0,"startedAt":"2026-10-29T00:00:00Z"}).to_string()).unwrap();
        let data = read(&s, &job, 3).unwrap();
        assert_eq!(data["runs"].as_array().unwrap().len(), 2);
        assert!(data["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["prompt"] == "原始提示词"));
        assert!(data["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["prompt"] == "本次提示词" && r["system"] == "系统规则"));
        assert!(!data.to_string().contains("private-provider-error"));
        let run = data["runs"].as_array().unwrap().iter().find(|r|r["id"]=="12-3-2").unwrap();
        assert_eq!(run["execution"]["pid"],42);
        assert_eq!(run["execution"]["exitCode"],0);
        assert!(run["stdout"]["bytes"].as_u64().unwrap()>0);
        assert!(run["stdout"]["updatedAt"].is_string());
        assert!(run["stderr"]["bytes"].as_u64().unwrap()>0);
        assert!(data["runs"].as_array().unwrap().iter().any(|r|r["execution"].is_null()));
        assert!(read(&s, &job, 4).is_err());
        fs::write(
            root.join("12-3-2/stdout.ndjson"),
            vec![b'a'; LOG_LIMIT as usize + 1],
        )
        .unwrap();
        assert!(read(&s, &job, 3).unwrap()["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| !r["error"].as_str().unwrap().is_empty()));
        drop(s);
        fs::remove_dir_all(path).unwrap();
    }
}
