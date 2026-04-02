use clap::Parser;
use glob::glob;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "taiji-log",
    about = "Taiji 日志过滤与 RAW 分组查看工具",
    after_help = "示例:\n  taiji-log logs/taji-2026-03-29.log -api /api/agent/v1/invoke -r <requestId> -t <threadId>\n  taiji-log logs/*.log -raw\n  cat logs/taji-2026-03-29.log | taiji-log -raw"
)]
struct Cli {
    #[arg(value_name = "INPUT")]
    inputs: Vec<String>,
    #[arg(long = "api", value_name = "API_PATH")]
    api: Option<String>,
    #[arg(short = 'r', long = "request-id", value_name = "REQUEST_ID")]
    request_id: Option<String>,
    #[arg(short = 't', long = "thread-id", value_name = "THREAD_ID")]
    thread_id: Option<String>,
    #[arg(long = "raw")]
    raw: bool,
    #[arg(long = "raw-f")]
    raw_f: bool,
    #[arg(long = "color", action = clap::ArgAction::SetTrue)]
    color: bool,
    #[arg(long = "no-color", action = clap::ArgAction::SetTrue)]
    no_color: bool,
}

#[derive(Clone, Debug)]
struct LogEntry {
    raw_line: String,
    message: String,
    timestamp: Option<String>,
    metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Default)]
struct RequestContext {
    timestamp: Option<String>,
    api: Option<String>,
    thread_id: Option<String>,
    status: Option<String>,
    duration: Option<String>,
}

#[derive(Clone, Debug)]
struct RawMessage {
    kind: RawKind,
    payload: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RawKind {
    Request,
    Response,
}

#[derive(Clone, Debug, Default)]
struct RawGroup {
    context: RequestContext,
    messages: Vec<RawMessage>,
}

fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn normalize_legacy_flags(args: Vec<String>) -> Vec<String> {
    args.into_iter()
        .map(|arg| match arg.as_str() {
            "-api" => "--api".to_string(),
            "-raw" => "--raw".to_string(),
            "-raw-f" => "--raw-f".to_string(),
            _ => arg,
        })
        .collect()
}

fn resolve_inputs(inputs: &[String]) -> Result<(Vec<PathBuf>, bool), String> {
    let mut paths = Vec::new();
    let mut use_stdin = false;

    for input in inputs {
        if input == "-" {
            use_stdin = true;
            continue;
        }
        if input.contains('*') || input.contains('?') || input.contains('[') {
            let mut matched = false;
            for entry in glob(input).map_err(|e| format!("通配符无效 {input}: {e}"))? {
                let path = entry.map_err(|e| format!("读取通配符结果失败: {e}"))?;
                if path.is_file() {
                    matched = true;
                    paths.push(path);
                }
            }
            if !matched {
                return Err(format!("未匹配到输入文件: {input}"));
            }
        } else {
            let path = PathBuf::from(input);
            if path.is_file() {
                paths.push(path);
            } else {
                return Err(format!("输入文件不存在: {input}"));
            }
        }
    }

    if paths.is_empty() && !io::stdin().is_terminal() {
        use_stdin = true;
    }

    if paths.is_empty() && !use_stdin {
        return Err("未检测到输入源，请提供日志文件或通过管道传入 stdin".to_string());
    }

    Ok((paths, use_stdin))
}

fn parse_line(line: &str) -> LogEntry {
    let trimmed = line.trim_end_matches('\n').to_string();
    let mut message = String::new();
    let mut timestamp = None;
    let mut metadata = Map::new();

    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&trimmed) {
        if let Some(Value::String(ts)) = obj.get("timestamp") {
            timestamp = Some(ts.clone());
        }
        if let Some(Value::String(msg)) = obj.get("message") {
            message = msg.clone();
        }
        if let Some(Value::Object(meta)) = obj.get("metadata") {
            metadata = meta.clone();
        }
    }
    if message.is_empty() {
        message = trimmed.clone();
    }

    LogEntry {
        raw_line: trimmed,
        message,
        timestamp,
        metadata,
    }
}

fn read_entries(paths: &[PathBuf], use_stdin: bool) -> Result<Vec<LogEntry>, String> {
    let mut entries = Vec::new();

    for path in paths {
        let file = File::open(path).map_err(|e| format!("无法打开文件 {}: {e}", path.display()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.map_err(|e| format!("读取文件 {} 失败: {e}", path.display()))?;
            entries.push(parse_line(&line));
        }
    }

    if use_stdin {
        let stdin = io::stdin();
        let reader = stdin.lock();
        for line in reader.lines() {
            let line = line.map_err(|e| format!("读取 stdin 失败: {e}"))?;
            entries.push(parse_line(&line));
        }
    }

    Ok(entries)
}

fn metadata_str<'a>(metadata: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    metadata.get(key)?.as_str()
}

fn url_to_path(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let no_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    if no_query.starts_with('/') {
        return no_query.to_string();
    }
    if let Some(scheme_index) = no_query.find("://") {
        let rest = &no_query[scheme_index + 3..];
        if let Some(path_index) = rest.find('/') {
            let path = &rest[path_index..];
            return path.to_string();
        }
        return "/".to_string();
    }
    if no_query.is_empty() {
        "/".to_string()
    } else {
        no_query.to_string()
    }
}

fn matches_filters(entry: &LogEntry, cli: &Cli) -> bool {
    if let Some(api_filter) = &cli.api {
        let target_api = url_to_path(api_filter);
        let Some(url) = metadata_str(&entry.metadata, "url") else {
            return false;
        };
        if url_to_path(url) != target_api {
            return false;
        }
    }
    if let Some(request_id_filter) = &cli.request_id
        && metadata_str(&entry.metadata, "requestId") != Some(request_id_filter.as_str())
    {
        return false;
    }
    if let Some(thread_id_filter) = &cli.thread_id
        && metadata_str(&entry.metadata, "threadId") != Some(thread_id_filter.as_str())
    {
        return false;
    }
    true
}

fn remove_size_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("size");
            for nested in map.values_mut() {
                remove_size_fields(nested);
            }
        }
        Value::Array(items) => {
            for nested in items {
                remove_size_fields(nested);
            }
        }
        _ => {}
    }
}

fn parse_raw_payload(metadata: &Map<String, Value>, kind: &RawKind) -> Option<Value> {
    let key = match kind {
        RawKind::Request => "modelRawRequest",
        RawKind::Response => "modelRawResponse",
    };
    let raw = metadata.get(key)?.as_str()?;
    let mut parsed =
        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
    remove_size_fields(&mut parsed);
    Some(parsed)
}

fn extract_request_context(entry: &LogEntry) -> (Option<String>, RequestContext) {
    let request_id = metadata_str(&entry.metadata, "requestId").map(ToString::to_string);
    let api = metadata_str(&entry.metadata, "url").map(url_to_path);
    let thread_id = metadata_str(&entry.metadata, "threadId").map(ToString::to_string);
    let status = entry
        .metadata
        .get("statusCode")
        .and_then(Value::as_i64)
        .map(|n| n.to_string());
    let duration = entry
        .metadata
        .get("duration")
        .and_then(Value::as_i64)
        .map(|n| format!("{n}ms"));
    (
        request_id,
        RequestContext {
            timestamp: entry.timestamp.clone(),
            api,
            thread_id,
            status,
            duration,
        },
    )
}

fn extract_roles(payload: &Value) -> Vec<String> {
    payload
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter_map(|item| {
                    item.get("role")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let mut current = String::new();
        let mut count = 0usize;
        for ch in raw_line.chars() {
            current.push(ch);
            count += 1;
            if count >= width {
                out.push(current);
                current = String::new();
                count = 0;
            }
        }
        if current.is_empty() {
            if raw_line.is_empty() {
                out.push(String::new());
            }
        } else {
            out.push(current);
        }
    }
    if out.is_empty() {
        vec![String::new()]
    } else {
        out
    }
}

fn push_wrapped(lines: &mut Vec<String>, prefix: &str, text: &str, width: usize) {
    let available = width.saturating_sub(prefix.chars().count()).max(20);
    let wrapped = wrap_text(text, available);
    for line in wrapped {
        lines.push(format!("{prefix}{line}"));
    }
}

fn pretty_json_or_string(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn render_request_focus(payload: &Value, lines: &mut Vec<String>, width: usize, color: bool) {
    lines.push(paint(color, "1;32", "RAW REQUEST FOCUS:"));
    let Some(messages) = payload.get("messages").and_then(Value::as_array) else {
        lines.push(format!("{} -", paint(color, "1;36", "messages:")));
        return;
    };
    lines.push(format!(
        "{} {}",
        paint(color, "1;36", "messages:"),
        messages.len()
    ));
    for (index, message) in messages.iter().enumerate() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let role_colored = match role {
            "system" => paint(color, "32", role),
            "user" => paint(color, "36", role),
            "assistant" => paint(color, "33", role),
            "tool" => paint(color, "35", role),
            _ => role.to_string(),
        };
        lines.push(format!(
            "{} role={}",
            paint(color, "1", &format!("msg[{index}]")),
            role_colored
        ));
        let Some(content) = message.get("content") else {
            lines.push(format!("  {} -", paint(color, "1;36", "content:")));
            continue;
        };
        match content {
            Value::String(text) => {
                push_wrapped(lines, "  ", text, width);
            }
            Value::Array(blocks) => {
                for block in blocks {
                    let block_type = block
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    match block_type {
                        "text" => {
                            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                            push_wrapped(lines, "  [text] ", text, width);
                        }
                        "tool_use" => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("-");
                            let id = block.get("id").and_then(Value::as_str).unwrap_or("-");
                            lines.push(format!(
                                "  {} name={} id={}",
                                paint(color, "1;34", "[tool_use]"),
                                name,
                                id
                            ));
                            if let Some(input) = block.get("input") {
                                lines.push(format!("    {}", paint(color, "1;36", "input:")));
                                for line in pretty_json_or_string(input).lines() {
                                    lines.push(format!("      {line}"));
                                }
                            }
                        }
                        "tool_result" => {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or("-");
                            lines.push(format!(
                                "  {} tool_use_id={}",
                                paint(color, "1;35", "[tool_result]"),
                                tool_use_id
                            ));
                            if let Some(result_content) = block.get("content") {
                                match result_content {
                                    Value::Array(items) => {
                                        for item in items {
                                            if let Some(text) = item.get("text").and_then(Value::as_str)
                                            {
                                                push_wrapped(lines, "    ", text, width);
                                            } else {
                                                for line in pretty_json_or_string(item).lines() {
                                                    lines.push(format!("    {line}"));
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        for line in pretty_json_or_string(result_content).lines() {
                                            lines.push(format!("    {line}"));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {
                            lines.push(format!("  [{block_type}]"));
                            for line in pretty_json_or_string(block).lines() {
                                lines.push(format!("    {line}"));
                            }
                        }
                    }
                }
            }
            _ => {
                for line in pretty_json_or_string(content).lines() {
                    lines.push(format!("  {line}"));
                }
            }
        }
    }
}

fn render_response_focus_with_color(
    payload: &Value,
    lines: &mut Vec<String>,
    width: usize,
    color: bool,
) {
    lines.push(paint(color, "1;35", "RAW RESPONSE FOCUS:"));
    if let Some(usage) = payload.get("usage").and_then(Value::as_object) {
        let prompt = usage
            .get("prompt_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let total = usage.get("total_tokens").and_then(Value::as_i64).unwrap_or(0);
        lines.push(format!(
            "{} total={} prompt={} completion={}",
            paint(color, "1;36", "tokens:"),
            total,
            prompt,
            completion
        ));
    }
    let Some(choices) = payload.get("choices").and_then(Value::as_array) else {
        lines.push(format!("{} -", paint(color, "1;36", "choices:")));
        return;
    };
    lines.push(format!(
        "{} {}",
        paint(color, "1;36", "choices:"),
        choices.len()
    ));
    for (index, choice) in choices.iter().enumerate() {
        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("-");
        lines.push(format!(
            "{} finish_reason={}",
            paint(color, "1", &format!("choice[{index}]")),
            finish_reason
        ));
        let message = choice.get("message").and_then(Value::as_object);
        if let Some(content) = message
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            && !content.is_empty()
        {
            push_wrapped(lines, "  ", content, width);
        } else {
            lines.push(format!("  {}", paint(color, "2", "[no content]")));
        }
        if let Some(tool_calls) = message
            .and_then(|m| m.get("tool_calls"))
            .and_then(Value::as_array)
            && !tool_calls.is_empty()
        {
            lines.push(format!("  {}", paint(color, "1;34", "tool_calls:")));
            for call in tool_calls {
                let id = call.get("id").and_then(Value::as_str).unwrap_or("-");
                let call_type = call.get("type").and_then(Value::as_str).unwrap_or("-");
                let function = call.get("function").and_then(Value::as_object);
                let name = function
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                lines.push(format!(
                    "    - {} id={} type={} name={}",
                    paint(color, "34", "tool_call"),
                    id,
                    call_type,
                    name
                ));
                if let Some(arguments) = function
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                {
                    let pretty_args = serde_json::from_str::<Value>(arguments)
                        .map(|v| pretty_json_or_string(&v))
                        .unwrap_or_else(|_| arguments.to_string());
                    lines.push(format!("      {}", paint(color, "1;36", "arguments:")));
                    for line in pretty_args.lines() {
                        lines.push(format!("        {line}"));
                    }
                }
            }
        }
    }
}

fn render_raw(entries: &[LogEntry], focus: bool, color: bool) -> String {
    let mut contexts: HashMap<String, RequestContext> = HashMap::new();
    let mut groups: HashMap<String, RawGroup> = HashMap::new();
    let mut ordered_group_ids = Vec::new();
    let mut current_request_id: Option<String> = None;
    let mut raw_message_count = 0usize;

    for entry in entries {
        let (request_id, context) = extract_request_context(entry);
        if let Some(rid) = request_id {
            current_request_id = Some(rid.clone());
            contexts.insert(rid, context);
        }

        let kind = match entry.message.as_str() {
            "RAW REQUEST" => Some(RawKind::Request),
            "RAW RESPONSE" => Some(RawKind::Response),
            _ => None,
        };
        let Some(kind) = kind else {
            continue;
        };

        let group_id = metadata_str(&entry.metadata, "requestId")
            .map(ToString::to_string)
            .or_else(|| current_request_id.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let Some(payload) = parse_raw_payload(&entry.metadata, &kind) else {
            continue;
        };

        if !groups.contains_key(&group_id) {
            ordered_group_ids.push(group_id.clone());
        }

        let group = groups.entry(group_id.clone()).or_default();
        if let Some(context) = contexts.get(&group_id) {
            group.context = context.clone();
        }
        if group.context.timestamp.is_none() {
            group.context.timestamp = entry.timestamp.clone();
        }
        group.messages.push(RawMessage { kind, payload });
        raw_message_count += 1;
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "{} {}, {} {}",
        paint(color, "1;33", "groups:"),
        groups.len(),
        paint(color, "1;33", "raw_messages:"),
        raw_message_count
    ));

    for group_id in ordered_group_ids {
        let Some(group) = groups.get(&group_id) else {
            continue;
        };
        lines.push(String::new());
        lines.push(paint(
            color,
            "1;33",
            &format!("===== requestId: {group_id} ====="),
        ));
        lines.push(format!(
            "{} {}",
            paint(color, "1;36", "time:"),
            group
                .context
                .timestamp
                .clone()
                .unwrap_or_else(|| "-".to_string())
        ));
        lines.push(format!(
            "{} {}",
            paint(color, "1;36", "api:"),
            group.context.api.clone().unwrap_or_else(|| "-".to_string())
        ));
        lines.push(format!(
            "{} {}",
            paint(color, "1;36", "requestId:"),
            if group_id.is_empty() { "-" } else { &group_id }
        ));
        lines.push(format!(
            "{} {}",
            paint(color, "1;36", "threadId:"),
            group
                .context
                .thread_id
                .clone()
                .unwrap_or_else(|| "-".to_string())
        ));
        lines.push(format!(
            "{} {}",
            paint(color, "1;36", "status:"),
            group
                .context
                .status
                .clone()
                .unwrap_or_else(|| "-".to_string())
        ));
        lines.push(format!(
            "{} {}",
            paint(color, "1;36", "duration:"),
            group
                .context
                .duration
                .clone()
                .unwrap_or_else(|| "-".to_string())
        ));

        for message in &group.messages {
            match message.kind {
                RawKind::Request => {
                    let roles = extract_roles(&message.payload);
                    lines.push(paint(color, "1;32", "RAW REQUEST roles:"));
                    if roles.is_empty() {
                        lines.push(paint(color, "2", "-"));
                    } else {
                        for role in roles {
                            let colored = match role.as_str() {
                                "system" => paint(color, "32", &role),
                                "user" => paint(color, "36", &role),
                                "assistant" => paint(color, "33", &role),
                                "tool" => paint(color, "35", &role),
                                _ => role,
                            };
                            lines.push(colored);
                        }
                    }
                    if focus {
                        render_request_focus(&message.payload, &mut lines, 100, color);
                    } else {
                        lines.push(paint(color, "1;32", "RAW REQUEST:"));
                        let pretty = serde_json::to_string_pretty(&message.payload)
                            .unwrap_or_else(|_| "\"<invalid json>\"".to_string());
                        lines.push(pretty);
                    }
                }
                RawKind::Response => {
                    if focus {
                        render_response_focus_with_color(&message.payload, &mut lines, 100, color);
                    } else {
                        lines.push(paint(color, "1;35", "RAW RESPONSE:"));
                        let pretty = serde_json::to_string_pretty(&message.payload)
                            .unwrap_or_else(|_| "\"<invalid json>\"".to_string());
                        lines.push(pretty);
                    }
                }
            }
        }
    }

    lines.join("\n")
}

fn run(cli: Cli) -> Result<String, String> {
    let (paths, use_stdin) = resolve_inputs(&cli.inputs)?;
    let entries = read_entries(&paths, use_stdin)?;
    let filtered = entries
        .into_iter()
        .filter(|entry| matches_filters(entry, &cli))
        .collect::<Vec<_>>();

    if cli.raw || cli.raw_f {
        let color = if cli.no_color {
            false
        } else if cli.color {
            true
        } else {
            io::stdout().is_terminal()
        };
        return Ok(render_raw(&filtered, cli.raw_f, color));
    }

    Ok(filtered
        .into_iter()
        .map(|entry| entry.raw_line)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn main() {
    let args = normalize_legacy_flags(std::env::args().collect());
    let cli = Cli::parse_from(args);
    match run(cli) {
        Ok(output) => {
            if !output.is_empty() {
                let mut stdout = io::stdout().lock();
                if let Err(err) = writeln!(stdout, "{output}")
                    && err.kind() != io::ErrorKind::BrokenPipe
                {
                    eprintln!("错误: 输出失败: {err}");
                    std::process::exit(1);
                }
            }
        }
        Err(err) => {
            eprintln!("错误: {err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_entry(message: &str, timestamp: &str, metadata: Value) -> LogEntry {
        let metadata = metadata.as_object().cloned().unwrap_or_default();
        LogEntry {
            raw_line: String::new(),
            message: message.to_string(),
            timestamp: Some(timestamp.to_string()),
            metadata,
        }
    }

    #[test]
    fn test_url_to_path() {
        assert_eq!(
            url_to_path("/api/agent/v1/invoke?a=1"),
            "/api/agent/v1/invoke"
        );
        assert_eq!(
            url_to_path("https://example.com/api/agent/v1/invoke?a=1#x"),
            "/api/agent/v1/invoke"
        );
        assert_eq!(url_to_path("https://example.com"), "/");
    }

    #[test]
    fn test_and_filters() {
        let cli = Cli {
            inputs: vec![],
            api: Some("/api/agent/v1/invoke".to_string()),
            request_id: Some("rid-1".to_string()),
            thread_id: Some("tid-1".to_string()),
            raw: false,
            raw_f: false,
            color: false,
            no_color: false,
        };
        let entry = build_entry(
            "Request started",
            "2026-01-01T00:00:00+08:00",
            serde_json::json!({
                "url": "/api/agent/v1/invoke?a=1",
                "requestId": "rid-1",
                "threadId": "tid-1"
            }),
        );
        assert!(matches_filters(&entry, &cli));
    }

    #[test]
    fn test_raw_roles_keep_order_and_duplicates() {
        let entries = vec![
            build_entry(
                "Agent invoke started",
                "2026-03-29T18:04:16.457+08:00",
                serde_json::json!({
                    "requestId": "rid-1",
                    "threadId": "tid-1",
                    "url": "/api/agent/v1/invoke",
                    "statusCode": 200,
                    "duration": 123
                }),
            ),
            build_entry(
                "RAW REQUEST",
                "2026-03-29T18:04:16.524+08:00",
                serde_json::json!({
                    "modelRawRequest": "{\"messages\":[{\"role\":\"system\"},{\"role\":\"user\"},{\"role\":\"user\"},{\"role\":\"assistant\"}],\"size\":10}"
                }),
            ),
            build_entry(
                "RAW RESPONSE",
                "2026-03-29T18:04:17.508+08:00",
                serde_json::json!({
                    "modelRawResponse": "{\"output\":\"ok\",\"size\":99}"
                }),
            ),
        ];

        let output = render_raw(&entries, false, false);
        assert!(output.contains("groups: 1, raw_messages: 2"));
        assert!(output.contains("system\nuser\nuser\nassistant"));
        assert!(!output.contains("\"size\""));
    }

    #[test]
    fn test_raw_focus_contains_role_and_content() {
        let entries = vec![build_entry(
            "RAW REQUEST",
            "2026-03-29T18:04:16.524+08:00",
            serde_json::json!({
                "requestId": "rid-2",
                "modelRawRequest": "{\"messages\":[{\"role\":\"user\",\"content\":\"line1\\nline2\"},{\"role\":\"assistant\",\"content\":\"ok\"}]}"
            }),
        )];

        let output = render_raw(&entries, true, false);
        assert!(output.contains("RAW REQUEST FOCUS:"));
        assert!(output.contains("msg[0] role=user"));
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
    }
}
