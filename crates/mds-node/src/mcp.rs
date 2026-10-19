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
        { "name": "search", "description": "Быстрый поиск по индексу этой ноды. Параметр kind сужает выдачу по метке: docs, goods, images, video, channel, news, gov, jobs, services, medicine, shop, haul, auto, realty, build, edu, ads, ai. Параметр region задаёт регион или район. Параметры lat, lon, radius_km поднимают наверх точки в радиусе от пользователя.", "inputSchema": { "type": "object", "properties": { "q": { "type": "string" }, "kind": { "type": "string" }, "region": { "type": "string" }, "lat": { "type": "number" }, "lon": { "type": "number" }, "radius_km": { "type": "number" } }, "required": ["q"] } },
        { "name": "geo_places", "description": "Геоаналитика и выгрузка организаций с открытыми контактами (телефоны, email, соцсети, мессенджеры) и координатами. Поддерживает поиск по кругу (lat, lon, radius_km), по квадрату координат (min_lat, max_lat, min_lon, max_lon), по региону/району/городу (region, place), по тематике (kind) и слову (q). Возвращает точное число найденных организаций (total_sites), число страниц (matched_pages) и список карточек.", "inputSchema": { "type": "object", "properties": { "q": { "type": "string" }, "kind": { "type": "string" }, "region": { "type": "string" }, "place": { "type": "string" }, "lat": { "type": "number" }, "lon": { "type": "number" }, "radius_km": { "type": "number" }, "min_lat": { "type": "number" }, "max_lat": { "type": "number" }, "min_lon": { "type": "number" }, "max_lon": { "type": "number" }, "limit": { "type": "integer" } } } },
        { "name": "query_stats", "description": "Ключевые запросы этой ноды", "inputSchema": { "type": "object" } },
        { "name": "site_positions", "description": "Место сайта в недавних запросах", "inputSchema": { "type": "object", "properties": { "host": { "type": "string" } }, "required": ["host"] } },
        { "name": "index_pages", "description": "Страницы своего индекса. Параметр kind отдаёт страницы с этой меткой.", "inputSchema": { "type": "object", "properties": { "host": { "type": "string" }, "kind": { "type": "string" }, "after": { "type": "integer" } } } },
        { "name": "kind_counts", "description": "Все метки системы (публичные и скрытые) и число страниц по каждой метке", "inputSchema": { "type": "object" } },
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
            let kind = args.get("kind").and_then(|value| value.as_str()).unwrap_or("");
            let region_arg = args.get("region").and_then(|value| value.as_str()).unwrap_or("");
            let mask = mds_types::tab_mask(kind);
            if !kind.trim().is_empty() && mask == 0 {
                return Err("метка не известна".into());
            }
            let geo = if !region_arg.trim().is_empty() {
                mds_types::geo_in(region_arg)
            } else {
                mds_types::geo_in(query)
            };
            if let Some(region) = geo.region {
                if let Some(map) = map {
                    if let Some(base) = map.hive_for(region) {
                        return Ok(json!({
                            "region": region,
                            "place": geo.place,
                            "hive": base,
                            "note": "ответ этого края лежит на его улье"
                        })
                        .to_string());
                    }
                }
            }
            let hits = engine
                .search_with_vector(query, mask, 0)
                .map_err(|err| err.to_string())?
                .0;
            let lat = args.get("lat").and_then(|v| v.as_f64());
            let lon = args.get("lon").and_then(|v| v.as_f64());
            let radius = args.get("radius_km").and_then(|v| v.as_f64());
            let around = match (lat, lon, radius) {
                (Some(la), Some(lo), Some(r)) if mds_types::valid_coords(la, lo) && r.is_finite() && r > 0.0 => {
                    Some((la, lo, r.clamp(0.1, 500.0)))
                }
                _ => None,
            };
            let ordered = mds_rank::prefer_geo_radius(hits, geo.region, geo.place, around);
            let in_radius_count = around.map(|(_, _, r)| {
                ordered
                    .iter()
                    .filter(|h| h.distance_km.map(|d| (d as f64) <= r).unwrap_or(false))
                    .count()
            });
            let short: Vec<_> = ordered
                .into_iter()
                .take(10)
                .map(|hit| {
                    let contacts = engine.contacts_for_url(&hit.url);
                    json!({
                        "url": hit.url,
                        "title": hit.title,
                        "snippet": hit.snippet,
                        "score": hit.score,
                        "region": hit.region,
                        "place": hit.place,
                        "distance_km": hit.distance_km,
                        "lat": hit.lat,
                        "lon": hit.lon,
                        "price": hit.price,
                        "kinds": mds_types::kind_codes(hit.kind),
                        "contacts": contacts
                    })
                })
                .collect();
            Ok(json!({
                "hits": short,
                "region": geo.region,
                "place": geo.place,
                "in_radius_count": in_radius_count
            })
            .to_string())
        }
        "geo_places" => {
            let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("");
            let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let mask = mds_types::tab_mask(kind);
            if !kind.trim().is_empty() && mask == 0 {
                return Err("метка не известна".into());
            }
            let region_raw = args.get("region").and_then(|v| v.as_str()).unwrap_or("");
            let place_raw = args.get("place").and_then(|v| v.as_str()).unwrap_or("");
            let geo_from_args = if !place_raw.trim().is_empty() {
                mds_types::geo_in(place_raw)
            } else if !region_raw.trim().is_empty() {
                mds_types::geo_in(region_raw)
            } else {
                mds_types::geo_in(q)
            };
            let req_region = if !region_raw.trim().is_empty() {
                mds_types::canonical_region(region_raw).or(geo_from_args.region)
            } else {
                geo_from_args.region
            };
            let req_place = if !place_raw.trim().is_empty() {
                mds_types::place_in(place_raw).or(geo_from_args.place)
            } else {
                geo_from_args.place
            };
            let lat = args.get("lat").and_then(|v| v.as_f64());
            let lon = args.get("lon").and_then(|v| v.as_f64());
            let radius = args.get("radius_km").and_then(|v| v.as_f64());
            let around = match (lat, lon, radius) {
                (Some(la), Some(lo), Some(r)) if mds_types::valid_coords(la, lo) && r.is_finite() && r > 0.0 => {
                    Some((la, lo, r.clamp(0.1, 500.0)))
                }
                _ => None,
            };
            let bbox = match (
                args.get("min_lat").and_then(|v| v.as_f64()),
                args.get("max_lat").and_then(|v| v.as_f64()),
                args.get("min_lon").and_then(|v| v.as_f64()),
                args.get("max_lon").and_then(|v| v.as_f64()),
            ) {
                (Some(min_la), Some(max_la), Some(min_lo), Some(max_lo))
                    if mds_types::valid_coords(min_la, min_lo)
                        && mds_types::valid_coords(max_la, max_lo)
                        && min_la <= max_la
                        && min_lo <= max_lo =>
                {
                    Some((min_la, max_la, min_lo, max_lo))
                }
                _ => None,
            };
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(50);
            let (total_sites, matched_pages, rows) = engine
                .geo_places(q, mask, req_region, req_place, around, bbox, limit)
                .map_err(|err| err.to_string())?;
            let items: Vec<_> = rows
                .into_iter()
                .map(|row| {
                    json!({
                        "host": row.host,
                        "url": row.url,
                        "title": row.title,
                        "snippet": row.snippet,
                        "region": row.region,
                        "place": row.place,
                        "lat": row.lat,
                        "lon": row.lon,
                        "distance_km": row.distance_km,
                        "price": row.price,
                        "kinds": mds_types::kind_codes(row.kind),
                        "contacts": row.contacts
                    })
                })
                .collect();
            Ok(json!({
                "total_sites": total_sites,
                "matched_pages": matched_pages,
                "region": req_region,
                "place": req_place,
                "places": items
            })
            .to_string())
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
            let kind = args.get("kind").and_then(|value| value.as_str()).unwrap_or("");
            let mask = mds_types::tab_mask(kind);
            if !kind.trim().is_empty() && mask == 0 {
                return Err("метка не известна".into());
            }
            let after = args.get("after").and_then(|value| value.as_i64()).unwrap_or(0);
            let filter = if mask == 0 { None } else { Some(mask) };
            let rows = engine.list_index(host, filter, after).map_err(|err| err.to_string())?;
            let next = rows.last().map(|row| row.id);
            let pages: Vec<_> = rows
                .into_iter()
                .map(|row| {
                    let contacts = engine.contacts_for_url(&row.url);
                    json!({
                        "id": row.id,
                        "url": row.url,
                        "title": row.title,
                        "host": row.host,
                        "kinds": mds_types::kind_codes(row.kind.max(0) as u32),
                        "contacts": contacts
                    })
                })
                .collect();
            Ok(json!({ "pages": pages, "next": next }).to_string())
        }
        "kind_counts" => {
            let rows = engine.kind_counts().map_err(|err| err.to_string())?;
            let kinds: Vec<_> = rows
                .into_iter()
                .map(|(code, label, place, count)| json!({ "code": code, "label": label, "place": place, "pages": count }))
                .collect();
            Ok(json!({ "kinds": kinds }).to_string())
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
