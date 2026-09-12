use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::errors::{ProtocolAction, ProtocolError, ProtocolErrorReason, ProtocolPhase, ProtocolResult};
use crate::event::{Event, EventType, ZERO_DIGEST_HEX};
use crate::event_log::AppendOnlyEventLog;
use crate::replay::ReplayEngine;

const VIEWER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>TRI-SYNC Viewer</title>
  <style>
    :root {
      --bg: #0b0f14;
      --panel: #111720;
      --border: #233043;
      --text: #ecf2ff;
      --muted: #9fb0c9;
      --ok: #2bd97f;
      --warn: #f5c451;
      --fail: #ff5c74;
      --line: #365072;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      padding: 18px;
      font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Arial, sans-serif;
      color: var(--text);
      background: var(--bg);
    }
    .header {
      display: flex;
      flex-wrap: wrap;
      justify-content: space-between;
      gap: 12px;
      margin-bottom: 14px;
    }
    .title { font-size: 1.1rem; font-weight: 700; letter-spacing: .02em; }
    .sub { color: var(--muted); font-size: .82rem; }
    .controls {
      display: flex;
      align-items: center;
      gap: 10px;
      color: var(--muted);
      font-size: .86rem;
    }
    select, button {
      background: var(--panel);
      color: var(--text);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 7px 10px;
      font-size: .85rem;
    }
    button { cursor: pointer; }
    .layout {
      display: grid;
      grid-template-columns: minmax(0, 2fr) minmax(320px, 1fr);
      gap: 14px;
    }
    .panel {
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 10px;
      padding: 12px;
      min-height: 320px;
    }
    .status {
      display: inline-block;
      font-size: .76rem;
      font-weight: 700;
      letter-spacing: .05em;
      text-transform: uppercase;
      padding: 3px 7px;
      border-radius: 999px;
      border: 1px solid transparent;
    }
    .status.valid { color: var(--ok); border-color: color-mix(in oklab, var(--ok) 50%, transparent); }
    .status.warning { color: var(--warn); border-color: color-mix(in oklab, var(--warn) 50%, transparent); }
    .status.failed { color: var(--fail); border-color: color-mix(in oklab, var(--fail) 50%, transparent); }
    .plane { border: 1px solid var(--border); border-radius: 10px; padding: 10px; margin-bottom: 10px; }
    .plane-head {
      display: flex;
      justify-content: space-between;
      gap: 8px;
      margin-bottom: 8px;
      font-size: .84rem;
    }
    .timeline { display: flex; flex-direction: column; gap: 8px; }
    .event {
      position: relative;
      border: 1px solid var(--border);
      border-radius: 9px;
      padding: 9px 10px 9px 14px;
      cursor: pointer;
      background: #0f1622;
    }
    .event::before {
      content: "";
      position: absolute;
      left: 6px;
      top: -9px;
      width: 2px;
      height: calc(100% + 18px);
      background: var(--line);
    }
    .event.start::before { top: 50%; height: 50%; }
    .event.end::before { height: 50%; }
    .event.broken::before {
      background: repeating-linear-gradient(to bottom, var(--fail) 0 3px, transparent 3px 6px);
    }
    .event.selected { border-color: #4f7fbf; box-shadow: 0 0 0 1px #4f7fbf44 inset; }
    .meta { display: flex; flex-wrap: wrap; gap: 8px; font-size: .76rem; color: var(--muted); margin-bottom: 5px; }
    .digests { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .73rem; color: #d7e6ff; word-break: break-all; }
    .failure {
      margin-top: 8px;
      border: 1px solid #6c2735;
      background: #23131a;
      border-radius: 8px;
      padding: 8px;
      font-size: .75rem;
      color: #ffccd5;
    }
    .failure-grid {
      margin-top: 6px;
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 8px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: .7rem;
      word-break: break-all;
    }
    .inspect { display: grid; gap: 8px; font-size: .83rem; color: var(--muted); }
    .kv { border-top: 1px solid var(--border); padding-top: 7px; }
    .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .75rem; color: #d7e6ff; word-break: break-all; }
    .empty { color: var(--muted); font-size: .85rem; padding: 8px; }
  </style>
</head>
<body>
  <div class="header">
    <div>
      <div class="title">TRI-SYNC Local Forensic Viewer</div>
      <div class="sub" id="logPath">read-only</div>
      <div class="sub" id="globalStatus"></div>
    </div>
    <div class="controls">
      <label>Namespace</label>
      <select id="namespaceSelect"></select>
      <label><input type="checkbox" id="compareToggle" /> compare planes</label>
      <button id="reloadBtn" type="button">Reload</button>
    </div>
  </div>
  <div class="layout">
    <div class="panel">
      <div id="timelineRoot"></div>
    </div>
    <div class="panel">
      <div class="title" style="font-size:.9rem;margin-bottom:10px;">Inspection</div>
      <div id="inspectRoot" class="inspect"><div class="empty">Select a node.</div></div>
    </div>
  </div>
  <script>
    let currentModel = null;
    let selected = null;

    function short(v) { return (v || "").slice(0, 16); }
    function esc(v) {
      return String(v ?? "").replace(/[&<>"]/g, s => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[s]));
    }
    function statusClass(s) { return s === "failed" ? "failed" : s === "warning" ? "warning" : "valid"; }

    async function loadModel() {
      const ns = document.getElementById("namespaceSelect").value || "";
      const compare = document.getElementById("compareToggle").checked ? "1" : "0";
      const query = new URLSearchParams();
      if (ns && ns !== "__all__") query.set("namespace", ns);
      query.set("compare", compare);
      const res = await fetch("/api/view?" + query.toString(), { method: "GET" });
      const data = await res.json();
      currentModel = data;
      render();
    }

    function render() {
      if (!currentModel) return;
      document.getElementById("logPath").textContent = currentModel.log_path + " · read-only mirror";
      const gs = currentModel.global_verification;
      document.getElementById("globalStatus").innerHTML =
        `<span class="status ${statusClass(gs.status)}">${esc(gs.status)}</span> ${esc(gs.message || "")}`;

      const select = document.getElementById("namespaceSelect");
      const prior = select.value || "__all__";
      select.innerHTML = `<option value="__all__">all</option>` +
        currentModel.namespaces.map(ns => `<option value="${esc(ns)}">${esc(ns)}</option>`).join("");
      select.value = currentModel.selected_namespace || prior || "__all__";
      if (!select.value) select.value = "__all__";
      document.getElementById("compareToggle").checked = currentModel.compare_planes;

      const root = document.getElementById("timelineRoot");
      if (!currentModel.planes.length) {
        root.innerHTML = `<div class="empty">No events found.</div>`;
        document.getElementById("inspectRoot").innerHTML = `<div class="empty">Select a node.</div>`;
        return;
      }
      root.innerHTML = currentModel.planes.map((plane, pIndex) => {
        return `<section class="plane">
          <div class="plane-head">
            <div><strong>${esc(plane.namespace)}</strong> · ${plane.events.length} events</div>
            <div><span class="status ${statusClass(plane.verification.status)}">${esc(plane.verification.status)}</span></div>
          </div>
          <div class="timeline">
            ${plane.events.map((ev, idx) => {
              const id = `${pIndex}:${idx}`;
              const broken = ev.status === "failed" ? "broken" : "";
              const start = idx === 0 ? "start" : "";
              const end = idx === plane.events.length - 1 ? "end" : "";
              const selectedClass = selected === id ? "selected" : "";
              return `<article class="event ${broken} ${start} ${end} ${selectedClass}" data-id="${id}">
                <div class="meta">
                  <span>#${ev.seq}</span>
                  <span>tick ${ev.tick}</span>
                  <span>${esc(ev.event_type)}</span>
                  <span class="status ${statusClass(ev.status)}">${esc(ev.status)}</span>
                </div>
                <div class="digests">prev ${esc(short(ev.prev_digest))} → digest ${esc(short(ev.digest))}</div>
                ${ev.failure ? `<div class="failure">
                  <div>${esc(ev.failure.reason || "verification failure")}</div>
                  <div class="failure-grid">
                    <div><strong>expected</strong><br>${esc(ev.failure.expected || "-")}</div>
                    <div><strong>actual</strong><br>${esc(ev.failure.actual || "-")}</div>
                  </div>
                </div>` : ""}
              </article>`;
            }).join("")}
          </div>
        </section>`;
      }).join("");

      root.querySelectorAll(".event").forEach(el => {
        el.addEventListener("click", () => {
          selected = el.getAttribute("data-id");
          renderInspect();
          render();
        });
      });
      renderInspect();
    }

    function renderInspect() {
      const inspect = document.getElementById("inspectRoot");
      if (!selected || !currentModel) {
        inspect.innerHTML = `<div class="empty">Select a node.</div>`;
        return;
      }
      const [p, e] = selected.split(":").map(v => parseInt(v, 10));
      const plane = currentModel.planes[p];
      const node = plane?.events[e];
      if (!node) {
        inspect.innerHTML = `<div class="empty">Select a node.</div>`;
        return;
      }
      const prev = e > 0 ? plane.events[e - 1] : null;
      const next = e + 1 < plane.events.length ? plane.events[e + 1] : null;
      inspect.innerHTML = `
        <div><span class="status ${statusClass(node.status)}">${esc(node.status)}</span></div>
        <div class="kv">seq <span class="mono">${node.seq}</span> · tick <span class="mono">${node.tick}</span></div>
        <div class="kv">type <span class="mono">${esc(node.event_type)}</span></div>
        <div class="kv">namespace <span class="mono">${esc(node.namespace)}</span></div>
        <div class="kv">key <span class="mono">${esc(node.key || "-")}</span></div>
        <div class="kv">root_digest <span class="mono">${esc(node.root_digest || "-")}</span></div>
        <div class="kv">timestamp_ms <span class="mono">${esc(node.timestamp_ms ?? "-")}</span></div>
        <div class="kv">error_code <span class="mono">${esc(node.error_code || "-")}</span></div>
        <div class="kv">offending_seq <span class="mono">${esc(node.offending_seq ?? "-")}</span></div>
        <div class="kv">prev digest <span class="mono">${esc(node.prev_digest)}</span></div>
        <div class="kv">digest <span class="mono">${esc(node.digest)}</span></div>
        <div class="kv">chain context</div>
        <div class="mono">previous: ${prev ? `#${prev.seq} ${short(prev.digest)}` : "-"}</div>
        <div class="mono">next: ${next ? `#${next.seq} ${short(next.digest)}` : "-"}</div>
      `;
    }

    document.getElementById("reloadBtn").addEventListener("click", loadModel);
    document.getElementById("namespaceSelect").addEventListener("change", loadModel);
    document.getElementById("compareToggle").addEventListener("change", loadModel);
    loadModel();
  </script>
</body>
</html>
"#;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ViewModel {
    pub log_path: String,
    pub read_only: bool,
    pub namespaces: Vec<String>,
    pub selected_namespace: Option<String>,
    pub compare_planes: bool,
    pub generated_at_ms: u64,
    pub global_verification: VerificationSummary,
    pub planes: Vec<NamespacePlane>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NamespacePlane {
    pub namespace: String,
    pub verification: VerificationSummary,
    pub first_break_seq: Option<u64>,
    pub events: Vec<VisualEvent>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerificationSummary {
    pub status: String,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub offending_seq: Option<u64>,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VisualEvent {
    pub seq: u64,
    pub tick: u64,
    pub event_type: String,
    pub namespace: String,
    pub key: Option<String>,
    pub root_digest: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub error_code: Option<String>,
    pub offending_seq: Option<u64>,
    pub detail: Option<String>,
    pub prev_digest: String,
    pub digest: String,
    pub status: String,
    pub failure: Option<FailureDetail>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FailureDetail {
    pub reason: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseKind {
    Json,
    Html,
    Text,
}

pub fn serve_local_viewer(log_path: &Path, host: &str, port: u16) -> ProtocolResult<()> {
    validate_log_path_jsonl(log_path)?;
    let listener = TcpListener::bind(format!("{host}:{port}")).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorReason::IoError,
            ProtocolPhase::Input,
            ProtocolAction::Reject,
            format!("could not bind local viewer on {host}:{port}: {err}"),
        )
    })?;

    println!("viewer_url=http://{host}:{port}");
    println!("viewer_mode=read-only");
    println!("viewer_log={}", log_path.display());

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("viewer_accept_error={err}");
                continue;
            }
        };
        if let Err(err) = handle_connection(&mut stream, log_path) {
            eprintln!("viewer_request_error={err}");
        }
    }

    Ok(())
}

pub fn validate_log_path_jsonl(log_path: &Path) -> ProtocolResult<()> {
    let ext = log_path.extension().and_then(|value| value.to_str());
    if ext != Some("jsonl") {
        return Err(ProtocolError::new(
            ProtocolErrorReason::UnsupportedFormat,
            ProtocolPhase::Input,
            ProtocolAction::Reject,
            format!(
                "viewer requires a .jsonl log path, got '{}'",
                log_path.display()
            ),
        )
        .with_expected(".jsonl")
        .with_actual(log_path.display().to_string()));
    }
    Ok(())
}

fn handle_connection(stream: &mut TcpStream, log_path: &Path) -> Result<(), String> {
    let mut buffer = [0_u8; 16 * 1024];
    let size = stream.read(&mut buffer).map_err(|err| err.to_string())?;
    if size == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buffer[..size]);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");

    if method != "GET" {
        let body = "{\"error\":\"read-only viewer accepts only GET\"}".to_string();
        write_response(stream, 405, ResponseKind::Json, &body)?;
        return Ok(());
    }

    let path = target.split('?').next().unwrap_or(target);
    if path == "/" {
        write_response(stream, 200, ResponseKind::Html, VIEWER_HTML)?;
        return Ok(());
    }
    if path == "/health" {
        write_response(stream, 200, ResponseKind::Text, "ok")?;
        return Ok(());
    }
    if path == "/api/view" {
        let namespace = query_param(target, "namespace");
        let compare = query_param(target, "compare").as_deref() == Some("1");
        match build_view_model(log_path, namespace.as_deref(), compare) {
            Ok(model) => {
                let body = serde_json::to_string(&model).map_err(|err| err.to_string())?;
                write_response(stream, 200, ResponseKind::Json, &body)?;
            }
            Err(err) => {
                let body =
                    serde_json::to_string(&err.to_json_value()).map_err(|ser| ser.to_string())?;
                write_response(stream, 400, ResponseKind::Json, &body)?;
            }
        }
        return Ok(());
    }

    write_response(stream, 404, ResponseKind::Text, "not found")
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    response_kind: ResponseKind,
    body: &str,
) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let content_type = match response_kind {
        ResponseKind::Json => "application/json; charset=utf-8",
        ResponseKind::Html => "text/html; charset=utf-8",
        ResponseKind::Text => "text/plain; charset=utf-8",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| err.to_string())?;
    stream.flush().map_err(|err| err.to_string())
}

pub fn build_view_model(
    log_path: &Path,
    namespace_filter: Option<&str>,
    compare_planes: bool,
) -> ProtocolResult<ViewModel> {
    validate_log_path_jsonl(log_path)?;
    let event_log = AppendOnlyEventLog::open(log_path);
    let events = event_log.load().map_err(|err| {
        ProtocolError::new(
            ProtocolErrorReason::IoError,
            ProtocolPhase::Replay,
            ProtocolAction::Halt,
            format!("could not load log {}: {err}", log_path.display()),
        )
    })?;
    Ok(build_view_model_from_events(
        log_path,
        &events,
        namespace_filter,
        compare_planes,
    ))
}

pub fn build_view_model_from_events(
    log_path: &Path,
    events: &[Event],
    namespace_filter: Option<&str>,
    compare_planes: bool,
) -> ViewModel {
    let mut namespaces: Vec<String> = BTreeSet::from_iter(events.iter().map(|e| e.namespace.clone()))
        .into_iter()
        .collect();
    if namespaces.is_empty() {
        namespaces.push("none".to_string());
    }

    let selected_namespace = namespace_filter.map(ToString::to_string);
    let plane_namespaces: Vec<String> = if compare_planes {
        namespaces
            .iter()
            .filter(|ns| *ns != "none")
            .cloned()
            .collect()
    } else if let Some(ns) = namespace_filter {
        vec![ns.to_string()]
    } else {
        namespaces
            .iter()
            .filter(|ns| *ns != "none")
            .take(1)
            .cloned()
            .collect()
    };

    let mut planes: Vec<NamespacePlane> = plane_namespaces
        .iter()
        .map(|namespace| {
            let plane_events: Vec<Event> = events
                .iter()
                .filter(|event| event.namespace == *namespace)
                .cloned()
                .collect();
            analyze_namespace_events(namespace, &plane_events)
        })
        .collect();

    if planes.is_empty() {
        planes.push(NamespacePlane {
            namespace: namespace_filter.unwrap_or("none").to_string(),
            verification: VerificationSummary {
                status: "valid".to_string(),
                error_code: None,
                message: Some("no events".to_string()),
                offending_seq: None,
                expected: None,
                actual: None,
            },
            first_break_seq: None,
            events: Vec::new(),
        });
    }

    let global_verification = if planes.iter().any(|plane| plane.verification.status == "failed") {
        planes
            .iter()
            .find(|plane| plane.verification.status == "failed")
            .map(|plane| plane.verification.clone())
            .unwrap_or_else(|| VerificationSummary {
                status: "failed".to_string(),
                error_code: None,
                message: Some("verification failed".to_string()),
                offending_seq: None,
                expected: None,
                actual: None,
            })
    } else if planes
        .iter()
        .any(|plane| plane.verification.status == "warning")
    {
        VerificationSummary {
            status: "warning".to_string(),
            error_code: None,
            message: Some("verified with warnings".to_string()),
            offending_seq: None,
            expected: None,
            actual: None,
        }
    } else {
        VerificationSummary {
            status: "valid".to_string(),
            error_code: None,
            message: Some("verified".to_string()),
            offending_seq: None,
            expected: None,
            actual: None,
        }
    };

    ViewModel {
        log_path: log_path.display().to_string(),
        read_only: true,
        namespaces: namespaces.into_iter().filter(|ns| ns != "none").collect(),
        selected_namespace,
        compare_planes,
        generated_at_ms: now_ms(),
        global_verification,
        planes,
    }
}

fn analyze_namespace_events(namespace: &str, events: &[Event]) -> NamespacePlane {
    let mut visual_events: Vec<VisualEvent> = events
        .iter()
        .map(|event| VisualEvent {
            seq: event.seq,
            tick: event.tick,
            event_type: event_type_label(event.event_type).to_string(),
            namespace: event.namespace.clone(),
            key: event.key.clone(),
            root_digest: event.root_digest.clone(),
            timestamp_ms: event.timestamp_ms,
            error_code: event.error_code.clone(),
            offending_seq: event.offending_seq,
            detail: event.detail.clone(),
            prev_digest: event.prev_digest.clone(),
            digest: event.digest.clone(),
            status: if event.event_type == EventType::ProtocolError {
                "warning".to_string()
            } else {
                "valid".to_string()
            },
            failure: None,
        })
        .collect();

    let mut expected_seq = 0_u64;
    let mut expected_prev_digest = ZERO_DIGEST_HEX.to_string();
    let mut break_index: Option<usize> = None;

    for (index, event) in events.iter().enumerate() {
        if event.seq != expected_seq {
            mark_failure(
                &mut visual_events[index],
                "SEQ_GAP".to_string(),
                Some(expected_seq.to_string()),
                Some(event.seq.to_string()),
            );
            break_index = Some(index);
            break;
        }
        if event.prev_digest != expected_prev_digest {
            mark_failure(
                &mut visual_events[index],
                "DIGEST_MISMATCH".to_string(),
                Some(expected_prev_digest.clone()),
                Some(event.prev_digest.clone()),
            );
            break_index = Some(index);
            break;
        }
        if let Err(_err) = event.validate_digest() {
            mark_failure(
                &mut visual_events[index],
                "DIGEST_MISMATCH".to_string(),
                event.expected_digest().ok(),
                Some(event.digest.clone()),
            );
            break_index = Some(index);
            break;
        }
        expected_seq = expected_seq.saturating_add(1);
        expected_prev_digest = event.digest.clone();
    }

    let replay = ReplayEngine::replay(events);
    let mut verification = match &replay {
        Ok(_) => VerificationSummary {
            status: if visual_events.iter().any(|event| event.status == "warning") {
                "warning".to_string()
            } else {
                "valid".to_string()
            },
            error_code: None,
            message: Some("replay OK".to_string()),
            offending_seq: None,
            expected: None,
            actual: None,
        },
        Err(err) => VerificationSummary {
            status: "failed".to_string(),
            error_code: Some(err.code().to_string()),
            message: Some(err.message.clone()),
            offending_seq: err.offending_seq.or(err.seq),
            expected: err.expected.clone(),
            actual: err.actual.clone(),
        },
    };

    if let Err(err) = replay {
        if break_index.is_none() {
            let target_seq = err.offending_seq.or(err.seq);
            let target_index = target_seq
                .and_then(|seq| visual_events.iter().position(|event| event.seq == seq))
                .unwrap_or(0);
            if let Some(event) = visual_events.get_mut(target_index) {
                mark_failure(
                    event,
                    err.code().to_string(),
                    err.expected.clone(),
                    err.actual.clone(),
                );
                break_index = Some(target_index);
            } else {
                verification.message = Some("replay failed".to_string());
            }
        }
    }

    NamespacePlane {
        namespace: namespace.to_string(),
        verification,
        first_break_seq: break_index.and_then(|idx| visual_events.get(idx).map(|event| event.seq)),
        events: visual_events,
    }
}

fn mark_failure(event: &mut VisualEvent, reason: String, expected: Option<String>, actual: Option<String>) {
    event.status = "failed".to_string();
    event.failure = Some(FailureDetail {
        reason,
        expected,
        actual,
    });
}

fn event_type_label(event_type: EventType) -> &'static str {
    match event_type {
        EventType::StateWrite => "STATE_WRITE",
        EventType::StateDelete => "STATE_DELETE",
        EventType::StateBatch => "STATE_BATCH",
        EventType::TickSeal => "TICK_SEAL",
        EventType::Compact => "COMPACT",
        EventType::ProtocolError => "PROTOCOL_ERROR",
    }
}

fn query_param(path_with_query: &str, key: &str) -> Option<String> {
    let query = path_with_query.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (raw_key, raw_value) = pair.split_once('=')?;
        if raw_key == key {
            Some(percent_decode(raw_value))
        } else {
            None
        }
    })
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let maybe = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(v) = maybe {
                    out.push(v as char);
                    i += 3;
                } else {
                    out.push('%');
                    i += 1;
                }
            }
            other => {
                out.push(other as char);
                i += 1;
            }
        }
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_map::BsmValue;

    #[test]
    fn viewer_rejects_non_jsonl_input() {
        let err = validate_log_path_jsonl(Path::new("/tmp/events.log")).unwrap_err();
        assert_eq!(err.code(), "UNSUPPORTED_FORMAT");
    }

    #[test]
    fn digest_mismatch_breaks_chain_at_exact_node() {
        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("v1".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first event");
        let mut second = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("v2".to_string()),
            false,
            first.digest.clone(),
            None,
        )
        .expect("second event");
        second.prev_digest = "deadbeef".repeat(8);
        let model = build_view_model_from_events(
            Path::new("/tmp/events.jsonl"),
            &[first, second],
            Some("tenant-a"),
            false,
        );
        let plane = &model.planes[0];
        assert_eq!(plane.first_break_seq, Some(1));
        assert_eq!(plane.events[1].status, "failed");
        assert_eq!(
            plane.events[1]
                .failure
                .as_ref()
                .map(|failure| failure.reason.as_str()),
            Some("DIGEST_MISMATCH")
        );
    }

    #[test]
    fn namespace_filter_and_compare_planes_are_isolated() {
        let a0 = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("a".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("a0");
        let b0 = Event::state_write(
            0,
            0,
            "tenant-b",
            "tenant-b:key",
            BsmValue::String("b".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("b0");
        let all = build_view_model_from_events(Path::new("/tmp/events.jsonl"), &[a0.clone(), b0.clone()], None, true);
        assert_eq!(all.planes.len(), 2);
        let filtered = build_view_model_from_events(
            Path::new("/tmp/events.jsonl"),
            &[a0, b0],
            Some("tenant-a"),
            false,
        );
        assert_eq!(filtered.planes.len(), 1);
        assert_eq!(filtered.planes[0].namespace, "tenant-a");
    }

    #[test]
    fn plane_status_matches_replay_result() {
        let first = Event::state_write(
            0,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("ok".to_string()),
            false,
            ZERO_DIGEST_HEX,
            None,
        )
        .expect("first");
        let mut invalid = Event::state_write(
            1,
            0,
            "tenant-a",
            "tenant-a:key",
            BsmValue::String("bad".to_string()),
            false,
            first.digest.clone(),
            None,
        )
        .expect("invalid");
        invalid.seq = 2;
        let model = build_view_model_from_events(
            Path::new("/tmp/events.jsonl"),
            &[first.clone(), invalid.clone()],
            Some("tenant-a"),
            false,
        );
        let replay_is_ok = ReplayEngine::replay(&[first, invalid]).is_ok();
        assert_eq!(replay_is_ok, model.planes[0].verification.status == "valid");
        assert_eq!(model.planes[0].verification.status, "failed");
    }
}
