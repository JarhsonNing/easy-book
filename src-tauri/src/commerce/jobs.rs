//! Durable local collection queue. One worker owns child processes; pause/cancel kills and waits.
use super::*;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

const SELECT: &str="SELECT json_object('id',id,'kind',kind,'title',title,'state',state,'stage',stage,'request',json(request),'result',json(result),'error',error,'processed',processed,'total',total,'createdAt',created_at,'updatedAt',updated_at) FROM collection_jobs";
const STATES: &[&str] = &["queued", "running", "waiting", "paused", "completed", "partial", "failed", "canceled"];

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageFilter {
    #[serde(default)]
    state: String,
    page: Option<i64>,
    page_size: Option<i64>,
}

pub(super) async fn page(
    State(s): State<AppState>,
    axum::extract::Query(f): axum::extract::Query<PageFilter>,
) -> Result<Json<Value>, ApiError> {
    if !f.state.is_empty() && !STATES.contains(&f.state.as_str()) {
        return Err(ApiError::bad("未知任务状态"));
    }
    let c = lock(&s);
    let size = f.page_size.unwrap_or(30).clamp(1, 100);
    let total: i64 = c.query_row(
        "SELECT count(*) FROM collection_jobs WHERE (?1='' OR state=?1)",
        [&f.state], |r| r.get(0),
    )?;
    let page = f.page.unwrap_or(1).max(1).min(((total + size - 1) / size).max(1));
    let items = rows(&c, &format!("{SELECT} WHERE (?1='' OR state=?1) ORDER BY id DESC LIMIT ?2 OFFSET ?3"),
        params![f.state, size, (page - 1) * size])?;
    let mut counts = json!({"all":0});
    for state in STATES { counts[*state] = json!(0); }
    for row in rows(&c,"SELECT json_object('state',state,'count',count(*)) FROM collection_jobs GROUP BY state",[])? {
        counts[text(&row,"state")] = row["count"].clone();
        counts["all"] = json!(counts["all"].as_i64().unwrap_or(0) + row["count"].as_i64().unwrap_or(0));
    }
    Ok(Json(json!({"items":items,"total":total,"page":page,"pageSize":size,"counts":counts})))
}
fn get(c: &Connection, id: i64) -> Result<Value, ApiError> {
    rows(c, &format!("{SELECT} WHERE id=?"), [id])?
        .pop()
        .ok_or_else(|| ApiError::bad("任务不存在"))
}
pub(super) async fn list(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(rows(
        &lock(&s),
        &format!("{SELECT} ORDER BY id DESC LIMIT 100"),
        []
    )?)))
}
pub(super) async fn detail(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(get(&lock(&s), id)?))
}
pub(super) async fn conversation(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String,String>>,
) -> Result<Json<Value>, ApiError> {
    let bid = query.get("bookId").and_then(|v|v.parse::<i64>().ok()).ok_or_else(||ApiError::bad("请选择图书"))?;
    let job = get(&lock(&s),id)?;
    tokio::task::spawn_blocking(move || crate::agents::ai::conversation::read(&s,&job,bid))
        .await.map_err(|e|ApiError::bad(e.to_string()))?.map(Json).map_err(ApiError::bad)
}
pub(super) async fn create(
    State(s): State<AppState>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let kind = required(&v, "kind")?;
    if kind=="ai"{return crate::agents::ai::enqueue(&s,&v).map(Json);}
    if !["catalog", "sources", "images", "publish"].contains(&kind.as_str()) {
        return Err(ApiError::bad("未知任务类型"));
    }
    let mut request = v.clone();
    let mut result = json!({});
    let mut title = text(&v, "title").to_string();
    let total;
    if kind == "catalog" {
        catalog_collect::build_collect_args(&request)?;
        let year = v["year"]
            .as_i64()
            .ok_or_else(|| ApiError::bad("请选择年份"))?;
        if !(2000..=2200).contains(&year) {
            return Err(ApiError::bad("年份无效"));
        }
        if title.is_empty() {
            title = format!("{} · {year}", text(&v["args"], "school"));
        }
        total = None;
    } else {
        let ids = v["bookIds"]
            .as_array()
            .ok_or_else(|| ApiError::bad("请选择图书"))?;
        if ids.is_empty() || ids.len() > 100 {
            return Err(ApiError::bad("每次选择 1–100 本图书"));
        }
        let c = lock(&s);
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        for id in ids {
            let id = id.as_i64().ok_or_else(|| ApiError::bad("图书编号无效"))?;
            if !seen.insert(id) {
                continue;
            }
            let b = db::get_book(&c, id)?.ok_or_else(|| ApiError::bad("图书不存在"))?;
            items.push(json!({"bookId":id,"title":b.title,"isbn":b.isbn,"state":"pending"}));
        }
        total = Some(items.len() as i64);
        if title.is_empty() {
            title = format!(
                "{} · {} 本",
                match kind.as_str() {
                    "sources" => "查找货源",
                    "images" => "补充图片",
                    _ => "准备发布",
                },
                items.len()
            );
        }
        request["bookIds"] = json!(items
            .iter()
            .map(|i| i["bookId"].clone())
            .collect::<Vec<_>>());
        result = json!({"items":items});
    }
    let c = lock(&s);
    let now = db::now();
    c.execute("INSERT INTO collection_jobs(kind,title,request,result,total,created_at,updated_at) VALUES(?,?,?,?,?,?,?)",params![kind,title,request.to_string(),result.to_string(),total,now,now])?;
    Ok(Json(get(&c, c.last_insert_rowid())?))
}
pub(super) async fn control(
    State(s): State<AppState>,
    Path((id, action)): Path<(i64, String)>,
    Json(v): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let c = lock(&s);
    let job = get(&c, id)?;
    let state = text(&job, "state");
    if action == "complete-item" {
        if state != "waiting" {
            return Err(ApiError::bad("任务当前不可确认"));
        }
        let bid = v["bookId"]
            .as_i64()
            .ok_or_else(|| ApiError::bad("请选择图书"))?;
        let kind = text(&job, "kind");
        let sql=match kind {"images"=>"SELECT EXISTS(SELECT 1 FROM images WHERE book_id=?)", "sources" =>"SELECT EXISTS(SELECT 1 FROM book_sources WHERE book_id=? AND json_extract(data,'$.status')='available' AND json_extract(data,'$.dropship')=1)", _=>"SELECT EXISTS(SELECT 1 FROM channel_products WHERE book_id=? UNION ALL SELECT 1 FROM listings WHERE book_id=?1)"};
        if !c.query_row(sql, [bid], |r| r.get::<_, bool>(0))? {
            return Err(ApiError::bad("尚未找到已保存的结果，请完成处理后再确认"));
        }
        let mut result = job["result"].clone();
        let items = result["items"]
            .as_array_mut()
            .ok_or_else(|| ApiError::bad("任务没有明细"))?;
        let item = items
            .iter_mut()
            .find(|i| i["bookId"] == bid)
            .ok_or_else(|| ApiError::bad("图书不属于此任务"))?;
        item["state"] = json!("done");
        let done = items.iter().filter(|i| i["state"] == "done").count();
        let finished = done == items.len();
        c.execute("UPDATE collection_jobs SET result=?,processed=?,state=?,stage=?,updated_at=? WHERE id=?",params![result.to_string(),done as i64,if finished{"completed"}else{"waiting"},if finished{"完成"}else{"等待处理"},db::now(),id])?;
    } else {
        let next = match action.as_str() {
            "pause" if ["queued", "running", "waiting"].contains(&state) => "paused",
            "resume" if state == "paused" => "queued",
            "retry" if ["failed", "partial"].contains(&state) => "queued",
            "cancel" if ["queued", "running", "waiting", "paused"].contains(&state) => "canceled",
            _ => return Err(ApiError::bad("当前状态不支持此操作")),
        };
        if action == "retry" && job["kind"] == "sources" {
            let mut result = job["result"].clone();
            if let Some(items) = result["items"].as_array_mut() {
                for item in items.iter_mut().filter(|i| i["state"] == "failed") { item["state"] = json!("pending"); }
            }
            c.execute("UPDATE collection_jobs SET result=? WHERE id=?",params![result.to_string(),id])?;
        }
        c.execute(
            "UPDATE collection_jobs SET state=?,error='',updated_at=? WHERE id=?",
            params![next, db::now(), id],
        )?;
    }
    Ok(Json(get(&c, id)?))
}

pub(super) async fn advance_sources(s: &AppState, job: &Value) -> Result<(), ApiError> {
    let id = job["id"].as_i64().unwrap();
    let mut result = job["result"].clone();
    let items = result["items"].as_array_mut().ok_or_else(||ApiError::bad("任务没有图书明细"))?;
    for item in items.iter_mut().filter(|i| i["state"] != "done" && i["state"] != "failed") {
        let bid = item["bookId"].as_i64().unwrap();
        let mut search = price_search::status(State(s.clone()), Path(bid)).await?.0;
        if search["state"] == "waiting" && (job["state"] == "queued" || text(&search,"error").contains("重启")) {
            search = price_search::control(State(s.clone()),Path(bid),Json(json!({"action":"resume","requestId":search["requestId"]}))).await?.0;
        }
        let search = if item["state"] == "pending" {
            if ["searching","waiting"].contains(&text(&search,"state")) { search }
            else { price_search::control(State(s.clone()),Path(bid),Json(json!({"action":"start"}))).await?.0 }
        } else { search };
        if search["requestId"] != item["requestId"] && item["state"] != "pending" {
            item["state"] = json!("failed"); item["error"] = json!("检索已被另一个任务替换，请重试"); continue;
        }
        item["requestId"] = search["requestId"].clone();
        item["searchUrl"] = search["searchUrl"].clone();
        item["error"] = search["error"].clone();
        if search["state"] == "completed" {
            item["state"] = json!("done"); item["count"] = search["count"].clone();
        } else if search["state"] == "canceled" {
            item["state"] = json!("failed"); item["error"] = json!("检索已取消");
        } else { item["state"] = json!("searching"); break; }
    }
    let done = items.iter().filter(|i| i["state"] == "done").count();
    let failed = items.iter().filter(|i| i["state"] == "failed").count();
    let complete = done + failed == items.len();
    let state = if !complete { "waiting" } else if failed == 0 { "completed" } else if done == 0 { "failed" } else { "partial" };
    lock(s).execute("UPDATE collection_jobs SET result=?,processed=?,state=?,stage=?,updated_at=? WHERE id=? AND state IN ('queued','waiting')",
        params![result.to_string(),(done+failed) as i64,state,if complete{"货源采集结束"}else{"等待淘宝采集，完成后自动检索下一本"},db::now(),id])?;
    Ok(())
}

pub fn start(s: AppState) {
    tokio::spawn(async move {
        loop {
            let next = {
                let c = lock(&s);
                rows(
                    &c,
                    &format!("{SELECT} WHERE (state='queued' AND (kind<>'ai' OR coalesce(json_extract(request,'$.notBefore'),0)<=unixepoch())) OR (kind='sources' AND state='waiting') ORDER BY CASE WHEN kind='sources' AND state='queued' THEN 2 WHEN state='queued' THEN 0 ELSE 1 END,id LIMIT 1"),
                    [],
                )
                .ok()
                .and_then(|mut v| v.pop())
            };
            if let Some(job) = next {
                let id = job["id"].as_i64().unwrap();
                if text(&job,"kind")=="ai"{
                    let claimed=lock(&s).execute("UPDATE collection_jobs SET state='running',stage='AI 准备资料',error='',updated_at=? WHERE id=? AND state='queued'",params![db::now(),id]).unwrap_or(0);
                    if claimed>0{
                        let worker=s.clone();
                        match tokio::task::spawn_blocking(move||crate::agents::ai::run_job(&worker,&job)).await{
                            Ok(Ok(Some(result)))=>{
                                let failed=result["errors"].as_array().map_or(0,Vec::len);let total=result["items"].as_array().map_or(0,Vec::len);
                                let state=if failed==0{"completed"}else if failed==total{"failed"}else{"partial"};
                                let _=lock(&s).execute("UPDATE collection_jobs SET state=?,stage=?,result=?,updated_at=? WHERE id=? AND state='running'",params![state,if failed==0{"草稿已保存"}else{"生成结束，部分结果待处理"},result.to_string(),db::now(),id]);
                            },Ok(Ok(None))=>{},Ok(Err(e))=>fail(&s,id,e),Err(e)=>fail(&s,id,e.to_string()),
                        }
                    }
                }else if text(&job,"kind") == "sources" {
                    if let Err(e) = advance_sources(&s,&job).await {
                        let _ = lock(&s).execute("UPDATE collection_jobs SET state='failed',error=?,updated_at=? WHERE id=? AND state IN ('queued','waiting')",params![e.message,db::now(),id]);
                    }
                }else if text(&job, "kind") != "catalog" {
                    let _=lock(&s).execute("UPDATE collection_jobs SET state='waiting',stage='等待处理',updated_at=? WHERE id=? AND state='queued'",params![db::now(),id]);
                } else {
                    let claimed=lock(&s).execute("UPDATE collection_jobs SET state='running',stage='获取官方目录',error='',updated_at=? WHERE id=? AND state='queued'",params![db::now(),id]).unwrap_or(0);
                    if claimed > 0 {
                        let ss = s.clone();
                        let outcome =
                            tokio::task::spawn_blocking(move || run_catalog(&ss, &job)).await;
                        match outcome {
                            Ok(Ok(Some(result))) => {
                                let imported = library::sync_catalog(State(s.clone())).await;
                                let mut result = result;
                                match imported {
                                    Ok(Json(v)) => result["import"] = v,
                                    Err(e) => {
                                        fail(&s, id, format!("入库失败：{e:?}"));
                                        continue;
                                    }
                                }
                                let partial =
                                    result["errors"].as_array().is_some_and(|v| !v.is_empty());
                                let _=lock(&s).execute("UPDATE collection_jobs SET state=?,stage='完成',result=?,updated_at=? WHERE id=? AND state='running'",params![if partial{"partial"}else{"completed"},result.to_string(),db::now(),id]);
                            }
                            Ok(Ok(None)) => {}
                            Ok(Err(e)) => fail(&s, id, e),
                            Err(e) => fail(&s, id, e.to_string()),
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
    });
}
fn fail(s: &AppState, id: i64, error: String) {
    let _=lock(s).execute("UPDATE collection_jobs SET state='failed',error=?,updated_at=? WHERE id=? AND state='running'",params![error,db::now(),id]);
}

pub(super) fn script_root() -> PathBuf {
    if let Ok(dir) = std::env::var("TAOSHU_COLLECTOR") {
        return PathBuf::from(dir).join("scripts");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/kaoyan")
}
pub(super) fn node() -> PathBuf {
    if let Ok(dir) = std::env::var("TAOSHU_COLLECTOR") {
        return PathBuf::from(dir).join(if cfg!(windows) { "node.exe" } else { "node" });
    }
    PathBuf::from("node")
}
fn run_command(
    s: &AppState,
    id: i64,
    script: &str,
    args: &[String],
    dir: &PathBuf,
) -> Result<Option<Value>, String> {
    let out = dir.join("stdout.json");
    let err = dir.join("stderr.log");
    let progress = dir.join("progress.json");
    let mut cmd = Command::new(node());
    cmd.arg(script_root().join(script))
        .args(args)
        .current_dir(&s.data_dir)
        .env("TAOSHU_COLLECT_DATA", s.data_dir.join("kaoyan"))
        .env("TAOSHU_PROGRESS", &progress)
        .env("TAOSHU_PARENT_PID", std::process::id().to_string())
        .stdout(Stdio::from(
            fs::File::create(&out).map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            fs::File::create(&err).map_err(|e| e.to_string())?,
        ));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("采集运行环境不可用：{e}"))?;
    loop {
        let state = lock(s)
            .query_row("SELECT state FROM collection_jobs WHERE id=?", [id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_default();
        if state != "running" {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        if let Ok(raw) = fs::read(&progress) {
            if let Ok(p) = serde_json::from_slice::<Value>(&raw) {
                let _=lock(s).execute("UPDATE collection_jobs SET processed=?,stage=?,updated_at=? WHERE id=? AND state='running'",params![p["processed"].as_i64().unwrap_or(0),text(&p,"stage"),db::now(),id]);
            }
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                if !status.success() {
                    let msg = fs::read_to_string(&err).unwrap_or_default();
                    return Err(msg
                        .chars()
                        .rev()
                        .take(1800)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect());
                }
                return serde_json::from_slice(&fs::read(&out).map_err(|e| e.to_string())?)
                    .map(Some)
                    .map_err(|e| format!("采集结果无效：{e}"));
            }
            None => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}
fn run_catalog(s: &AppState, job: &Value) -> Result<Option<Value>, String> {
    let id = job["id"].as_i64().unwrap();
    let dir = s.data_dir.join("jobs").join(id.to_string());
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let args =
        catalog_collect::build_collect_args(&job["request"]).map_err(|e| format!("{e:?}"))?;
    // Resume from successfully cached pages; a new explicit task can request a fresh fetch.
    let mut resume = job["request"].clone();
    resume["refresh"] = json!(false);
    lock(s)
        .execute(
            "UPDATE collection_jobs SET request=? WHERE id=?",
            params![resume.to_string(), id],
        )
        .map_err(|e| e.to_string())?;
    let Some(mut result) = run_command(s, id, "run-plugin.mjs", &args, &dir)? else {
        return Ok(None);
    };
    let _ = fs::remove_file(dir.join("progress.json"));
    lock(s).execute("UPDATE collection_jobs SET stage='整理用书目录',updated_at=? WHERE id=? AND state='running'",params![db::now(),id]).map_err(|e|e.to_string())?;
    let input = s.data_dir.join("kaoyan/books");
    let output = s
        .data_dir
        .join("kaoyan/catalog")
        .join(format!("{id}-{}", chrono::Utc::now().timestamp_millis()));
    let args = vec![
        "--input".into(),
        input.to_string_lossy().into(),
        "--out".into(),
        output.to_string_lossy().into(),
    ];
    let Some(build) = run_command(s, id, "build-catalog.mjs", &args, &dir)? else {
        return Ok(None);
    };
    result["catalog"] = build;
    if get(&lock(s), id).map_err(|e| format!("{e:?}"))?["state"] != "running" {
        return Ok(None);
    }
    db::set_setting(
        &lock(s),
        "kaoyanCatalogPath",
        &json!(output.join("catalog.db").to_string_lossy()),
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(result))
}
