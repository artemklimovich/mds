use serde_json::{json, Value};

use crate::Engine;

pub fn handle(engine: &mut Engine, map: Option<&mds_wire::NetworkMap>, body: &str, authed: bool) -> (u16, String) {
    if !authed {
        return (404, String::new());
    }
    let request: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return (400, rpc_error(None, -32700, "запрос не читается")),
    };
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(|value| value.as_str()).unwrap_or("");
    if method.starts_with("notifications/") {
        return (202, String::new());
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "mds", "version": mds_update::VERSION }
        }),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => match call(engine, map, request.get("params").unwrap_or(&Value::Null)) {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(message) => return (200, rpc_error(id, -32000, &message)),
        },
        _ => return (200, rpc_error(id, -32601, "метод не известен")),
    };
    (200, json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string())
}

fn tools() -> Value {
    json!([
        { "name": "search", "description": "Поиск по индексу этой ноды", "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } }, "required": ["q"] } },
        { "name": "query_stats", "description": "Ключевые запросы этой ноды", "inputSchema": { "type": "object" } },
        { "name": "site_positions", "description": "Место сайта в недавних запросах", "inputSchema": { "type": "object", "properties": { "host": { "type": "string" } }, "required": ["host"] } },
        { "name": "index_pages", "description": "Страницы своего индекса", "inputSchema": { "type": "object", "properties": { "host": { "type": "string" }, "after": { "type": "integer" } } } },
        { "name": "node_status", "description": "Состояние этой ноды", "inputSchema": { "type": "object" } },
        { "name": "visit_stats", "description": "Заходы, время, отказы и ошибки поиска без адресов людей", "inputSchema": { "type": "object" } },
        { "name": "network_map", "description": "Подписанная карта сети", "inputSchema": { "type": "object" } }
    ])
}

fn call(engine: &mut Engine, map: Option<&mds_wire::NetworkMap>, params: &Value) -> Result<String, String> {
    let name = params.get("name").and_then(|value| value.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match name {
        "search" => {
            let query = args.get("q").and_then(|value| value.as_str()).unwrap_or("");
            if let Some(region) = mds_types::region_in(query) {
                if let Some(map) = map {
                    if let Some(base) = map.hive_for(region) {
                        return Ok(json!({
                            "region": region,
                            "hive": base,
                            "note": "ответ этого края лежит на его улье"
                        })
                        .to_string());
                    }
                }
            }
            let hits = engine
                .search_with_vector(query, 0)
                .map_err(|err| err.to_string())?
                .0;
            let short: Vec<_> = hits
                .into_iter()
                .take(10)
                .map(|hit| json!({ "url": hit.url, "title": hit.title, "score": hit.score }))
                .collect();
            Ok(json!({ "hits": short }).to_string())
        }
        "query_stats" => {
            let day = engine.query_counts("day").map_err(|err| err.to_string())?;
            let week = engine.query_counts("week").map_err(|err| err.to_string())?;
            let month = engine.query_counts("month").map_err(|err| err.to_string())?;
            Ok(json!({ "day": day, "week": week, "month": month }).to_string())
        }
        "site_positions" => {
            let host = args.get("host").and_then(|value| value.as_str()).unwrap_or("");
            let rows = engine.site_places(host).map_err(|err| err.to_string())?;
            Ok(json!({ "places": rows }).to_string())
        }
        "index_pages" => {
            let host = args.get("host").and_then(|value| value.as_str());
            let after = args.get("after").and_then(|value| value.as_i64()).unwrap_or(0);
            let rows = engine.list_index(host, None, after).map_err(|err| err.to_string())?;
            let next = rows.last().map(|row| row.id);
            let pages: Vec<_> = rows
                .into_iter()
                .map(|row| json!({ "id": row.id, "url": row.url, "title": row.title, "host": row.host }))
                .collect();
            Ok(json!({ "pages": pages, "next": next }).to_string())
        }
        "visit_stats" => {
            let day = engine.visit_span("day").map_err(|err| err.to_string())?;
            let week = engine.visit_span("week").map_err(|err| err.to_string())?;
            let month = engine.visit_span("month").map_err(|err| err.to_string())?;
            Ok(json!({
                "day": visit_json(day),
                "week": visit_json(week),
                "month": visit_json(month)
            })
            .to_string())
        }
        "node_status" => {
            let (searches, avg_ms) = engine.load_snapshot();
            Ok(json!({
                "pages": engine.len().unwrap_or(0),
                "queue": engine.queue_len().unwrap_or(0),
                "db_bytes": engine.db_bytes(),
                "searches_per_minute": searches,
                "avg_ms": avg_ms,
                "version": mds_update::VERSION
            })
            .to_string())
        }
        "network_map" => Ok(serde_json::to_string(&map).unwrap_or_else(|_| "null".into())),
        _ => Err("инструмент не известен".into()),
    }
}

fn visit_json(report: mds_index::VisitReport) -> Value {
    json!({
        "visits": report.visits,
        "views": report.views,
        "searches": report.searches,
        "bounces": report.bounces,
        "dwell_seconds": report.dwell_seconds,
        "timed_visits": report.timed_visits,
        "empty_searches": report.empty_searches,
        "search_errors": report.search_errors,
        "refused": report.refused
    })
}

fn rpc_error(id: Option<Value>, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}
