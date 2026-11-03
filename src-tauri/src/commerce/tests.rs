use super::*;
use crate::models::{BookPatch, CaptureInput};
use std::{fs, path::PathBuf};

#[tokio::test]
async fn task_pages_filter_all_history_and_count_every_state() {
    let f = Fixture::new();
    let states = ["completed", "failed", "running", "waiting", "paused", "partial", "canceled", "queued"];
    {
        let c = lock(&f.state);
        for state in states.iter().chain(std::iter::repeat(&"queued").take(125)) {
            c.execute("INSERT INTO collection_jobs(kind,title,state,request,created_at,updated_at) VALUES('ai','任务',?,'{}','now','now')", [state]).unwrap();
        }
    }
    let query = |v:Value| jobs::page(State(f.state.clone()), axum::extract::Query(serde_json::from_value(v).unwrap()));
    let all = query(json!({})).await.unwrap().0;
    assert_eq!(all["total"],133);
    assert_eq!(all["items"].as_array().unwrap().len(),30);
    assert!(all["items"].as_array().unwrap().iter().all(|j|j["state"]=="queued"));
    assert_eq!(all["counts"]["queued"],126);
    assert_eq!(all["counts"]["all"],133);
    for state in states {
        let filtered = query(json!({"state":state})).await.unwrap().0;
        assert_eq!(filtered["total"], if state == "queued" {126} else {1});
        assert!(filtered["items"].as_array().unwrap().iter().all(|j|j["state"]==state));
        assert_eq!(filtered["counts"]["completed"],1);
    }
    let last = query(json!({"page":999})).await.unwrap().0;
    assert_eq!(last["page"],5);
    assert_eq!(last["items"].as_array().unwrap().len(),13);
    assert_eq!(last["items"].as_array().unwrap().last().unwrap()["id"],1);
    let bounded = query(json!({"page":0,"pageSize":1000})).await.unwrap().0;
    assert_eq!(bounded["page"],1); assert_eq!(bounded["pageSize"],100);
    assert!(query(json!({"state":"unknown"})).await.is_err());
    let detail = jobs::detail(State(f.state.clone()),Path(1)).await.unwrap().0;
    assert_eq!(detail["state"],"completed");
    let empty = Fixture::new();
    let result = jobs::page(State(empty.state.clone()),axum::extract::Query(serde_json::from_value(json!({"state":"completed","page":99})).unwrap())).await.unwrap().0;
    assert_eq!(result["total"],0); assert_eq!(result["page"],1); assert_eq!(result["counts"]["queued"],0);
}

#[tokio::test]
async fn textbook_classifications_filter_together_and_keep_facets_outside_the_page() {
    let f=Fixture::new();
    let (a,b,blank)={
        let c=lock(&f.state);
        let create=|title:&str,level:&str,major:&str,category:&str| db::create_book(&c,&serde_json::from_value(json!({"title":title,"textbookLevel":level,"applicableMajor":major,"category":category,"stock":0})).unwrap()).unwrap().id;
        let a=create("软件技术教材","高职高专","软件技术","高职教育类");
        let b=create("组合分级教材","本科、高职","计算机","理工类");
        let blank=create("未分类书目","","","");
        let patch=serde_json::from_value(json!({"教材分级":"本科","适用专业":"法学","类别":"文科类"})).unwrap();
        db::update_book(&c,a,&patch).unwrap();db::update_book(&c,a,&patch).unwrap();
        db::init(&c).unwrap();
        assert_eq!(db::get_book(&c,a).unwrap().unwrap().textbook_classifications.len(),2);
        (a,b,blank)
    };
    for (filter,expected) in [
        (json!({"textbookLevel":"高职高专"}),vec![a]),
        (json!({"applicableMajor":"计算机"}),vec![b]),
        (json!({"category":"文科类"}),vec![a]),
        (json!({"textbookLevel":"本科、高职","applicableMajor":"计算机","category":"理工类"}),vec![b]),
        (json!({"textbookLevel":"高职高专","category":"文科类"}),vec![]),
        (json!({"category":"__empty__"}),vec![blank]),
        (json!({"q":"软件","category":"文科类"}),vec![a]),
    ] {
        let response=library::list(State(f.state.clone()),axum::extract::Query(serde_json::from_value(filter).unwrap())).await.unwrap().0;
        let ids:Vec<i64>=response["items"].as_array().unwrap().iter().map(|b|b["id"].as_i64().unwrap()).collect();
        assert_eq!(ids,expected);
    }
    let page=library::list(State(f.state.clone()),axum::extract::Query(serde_json::from_value(json!({"pageSize":1})).unwrap())).await.unwrap().0;
    assert_eq!(page["total"],3);assert_eq!(page["items"].as_array().unwrap().len(),1);
    assert!(page["applicableMajors"].as_array().unwrap().contains(&json!("软件技术")));
}

#[tokio::test]
async fn batch_sources_starts_and_advances_only_after_collected_results() {
    let f=Fixture::new(); let a=f.book("第一本"); let b=f.book("第二本");
    let job=jobs::create(State(f.state.clone()),Json(json!({"kind":"sources","bookIds":[a,b]}))).await.unwrap().0;
    jobs::advance_sources(&f.state,&job).await.unwrap();
    let job=jobs::detail(State(f.state.clone()),Path(job["id"].as_i64().unwrap())).await.unwrap().0;
    assert_eq!(job["state"],"waiting"); assert_eq!(job["result"]["items"][0]["state"],"searching");
    assert!(job["result"]["items"][0]["searchUrl"].as_str().unwrap().contains("taoshuPriceSearch"));
    assert_eq!(job["result"]["items"][1]["state"],"pending");
    let req=job["result"]["items"][0]["requestId"].clone();
    let _=price_search::control(State(f.state.clone()),Path(a),Json(json!({"action":"batch","requestId":req,"batch":1,"items":[{"url":"https://item.taobao.com/item.htm?id=998","title":"教材","price":8,"shop":"小谷吖书店"}]}))).await.unwrap();
    let _=price_search::control(State(f.state.clone()),Path(a),Json(json!({"action":"finish","requestId":req,"reason":"visible-end"}))).await.unwrap();
    jobs::advance_sources(&f.state,&job).await.unwrap();
    let next=jobs::detail(State(f.state.clone()),Path(job["id"].as_i64().unwrap())).await.unwrap().0;
    assert_eq!(next["processed"],1); assert_eq!(next["result"]["items"][0]["state"],"done");
    assert_eq!(next["result"]["items"][1]["state"],"searching");
    let req=next["result"]["items"][1]["requestId"].clone();
    let _=price_search::control(State(f.state.clone()),Path(b),Json(json!({"action":"batch","requestId":req,"batch":1,"items":[{"url":"https://item.taobao.com/item.htm?id=999","title":"第二本教材","price":9}]}))).await.unwrap();
    let _=price_search::control(State(f.state.clone()),Path(b),Json(json!({"action":"finish","requestId":req,"reason":"visible-end"}))).await.unwrap();
    jobs::advance_sources(&f.state,&next).await.unwrap();
    let completed=jobs::detail(State(f.state.clone()),Path(job["id"].as_i64().unwrap())).await.unwrap().0;
    assert_eq!(completed["state"],"completed");assert_eq!(completed["processed"],2);
    let sources=source_list(&lock(&f.state),Some(a)).unwrap();
    assert_eq!(sources[0]["data"]["status"],"available");
    assert_eq!(sources[0]["data"]["freight"],0);
    assert_eq!(sources[0]["data"]["verificationMethod"],"shop-whitelist");
}

#[test]
fn whitelist_defaults_are_exact_configurable_and_need_a_price() {
    let f=Fixture::new();let bid=f.book("白名单");let c=lock(&f.state);
    let mut data=json!({"shop":"南工大书店","price":null,"status":"pending"});
    source_defaults(&c,bid,&mut data).unwrap();assert_eq!(data["status"],"pending");
    assert_eq!(data["isbn"],"9787300000001");assert_eq!(data["condition"],"九成新");
    data["price"]=json!(12);source_defaults(&c,bid,&mut data).unwrap();assert_eq!(data["status"],"available");
    let _=db::save_settings(&c,&json!({"sourceShopWhitelist":[]})).unwrap();
    let mut data=json!({"shop":"南工大书店","price":12,"status":"pending"});source_defaults(&c,bid,&mut data).unwrap();assert_eq!(data["status"],"pending");
    assert!(crate::listing::title_length(&crate::listing::limit_title(&"书ABC".repeat(30)))<=60);
    let defaults=db::settings(&c).xianyu_publish_defaults;
    assert!(crate::regions::valid(defaults.province,defaults.city,defaults.district));assert_eq!(defaults.stock,9999);
}

#[tokio::test]
async fn blank_edition_does_not_add_channel_blockers_or_missing_metadata_tasks() {
    let f = Fixture::new();
    let bid = f.book("版次可选的图书");
    f.channel(bid, "physical");
    {
        let c = lock(&f.state);
        let before = check_channel(&c, 1).unwrap();
        let patch = serde_json::from_value(json!({"edition":""})).unwrap();
        db::update_book(&c, bid, &patch).unwrap();
        let after = check_channel(&c, 1).unwrap();
        assert_eq!(before["blockers"], after["blockers"]);
        assert!(!after["blockers"].as_array().unwrap().is_empty());
        c.execute("INSERT INTO book_profiles(book_id,data) VALUES(?,?)", params![bid,json!({"confirmed":true}).to_string()]).unwrap();
    }
    let tasks = overview(State(f.state.clone())).await.unwrap().0;
    assert!(!tasks["tasks"].as_array().unwrap().iter().any(|t| text(t,"reason").contains("确认书目")));
}

#[tokio::test]
async fn primary_source_cost_tracks_saved_selection_updates_and_clearing() {
    let f=Fixture::new();let bid=f.book("主选成本同步");
    let mut input=json!({"bookId":bid,"primary":false,"data":{"url":"https://item.taobao.com/item.htm?id=123","status":"available","price":30.8,"freight":2,"sku":"第2版 单册","checkedAt":db::now()}});
    let saved=save_source(State(f.state.clone()),Json(input.clone())).await.unwrap().0;
    assert_eq!(db::get_book(&lock(&f.state),bid).unwrap().unwrap().cost_price,None);
    input["id"]=saved["id"].clone(); input["primary"]=json!(true);
    let _=save_source(State(f.state.clone()),Json(input.clone())).await.unwrap();
    assert_eq!(db::get_book(&lock(&f.state),bid).unwrap().unwrap().cost_price,Some(30.8));
    input["data"]["price"]=json!(28.5);
    let _=save_source(State(f.state.clone()),Json(input.clone())).await.unwrap();
    assert_eq!(db::get_book(&lock(&f.state),bid).unwrap().unwrap().cost_price,Some(28.5));
    input["primary"]=json!(false);
    let _=save_source(State(f.state.clone()),Json(input)).await.unwrap();
    let book=db::get_book(&lock(&f.state),bid).unwrap().unwrap();
    assert_eq!(book.cost_price,None);assert_eq!(book.sell_price,None);
}

#[tokio::test]
async fn primary_source_requires_verified_details_before_selection_changes() {
    let f=Fixture::new(); let bid=f.book("确认主选货源");
    let mut input=json!({"bookId":bid,"primary":true,"data":{"url":"https://item.taobao.com/item.htm?id=12","status":"pending","price":12,"freight":null,"dropship":false}});
    assert!(save_source(State(f.state.clone()),Json(input.clone())).await.is_err());
    assert!(source_list(&lock(&f.state),Some(bid)).unwrap().is_empty());
    input["data"]["status"]=json!("available");
    assert!(save_source(State(f.state.clone()),Json(input.clone())).await.is_err());
    input["data"]["sku"]=json!("第5版 上册"); input["data"]["checkedAt"]=json!(db::now());
    let saved=save_source(State(f.state.clone()),Json(input.clone())).await.unwrap().0;
    input["id"]=saved["id"].clone(); input["data"]["status"]=json!("pending");
    assert!(save_source(State(f.state.clone()),Json(input)).await.is_err());
    assert_eq!(source_list(&lock(&f.state),Some(bid)).unwrap()[0]["primary"],true);
    assert_eq!(source_list(&lock(&f.state),Some(bid)).unwrap()[0]["data"]["status"],"available");
}

#[tokio::test]
async fn source_links_are_atomic_deduplicated_and_enriched_without_overwriting_reviewed_data() {
    let f = Fixture::new(); let bid = f.book("货源整理测试");
    let import = |urls:Value| source_links::import(State(f.state.clone()),Path(bid),Json(json!({"urls":urls})));
    assert!(import(json!(["https://item.taobao.com/item.htm?id=123","https://example.com/?id=1"])).await.is_err());
    assert!(source_list(&lock(&f.state),Some(bid)).unwrap().is_empty());
    let first=import(json!(["https://item.taobao.com/item.htm?id=123&spm=tracking","https://item.taobao.com/item.htm?id=123"])).await.unwrap().0;
    assert_eq!(first["added"],1); assert_eq!(first["existing"],1);
    let sources=source_list(&lock(&f.state),Some(bid)).unwrap(); let sid=sources[0]["id"].as_i64().unwrap();
    assert_eq!(sources[0]["data"]["price"],Value::Null); assert_eq!(sources[0]["data"]["condition"],"");
    assert_eq!(sources[0]["data"]["status"],"pending"); assert_eq!(sources[0]["primary"],false);
    let mut item=json!({"url":"https://item.taobao.com/item.htm?id=123","title":"第5版上册","shop":"测试店铺","price":12});
    assert!(source_links::enrich(&lock(&f.state),bid,&item).unwrap());
    let capture:CaptureInput=serde_json::from_value(json!({"url":item["url"],"itemId":"123","title":item["title"],"price":12,"asSource":true,"sourceBookId":bid})).unwrap();
    let c=lock(&f.state); let (capture,_,_)=db::upsert_capture(&c,&capture,0.3,5.0,"九成新").unwrap();
    capture_candidate(&c,&capture).unwrap(); attach_capture_candidate(&c,&capture,bid).unwrap();
    assert_eq!(source_list(&c,Some(bid)).unwrap().len(),1);
    c.execute("UPDATE book_sources SET data=json_set(data,'$.price',10,'$.status','available','$.checkedAt','checked') WHERE id=?",[sid]).unwrap();
    item["price"]=json!(1); source_links::enrich(&c,bid,&item).unwrap();
    assert_eq!(source_list(&c,Some(bid)).unwrap()[0]["data"]["price"],10);
}

#[tokio::test]
async fn source_search_freezes_ai_configuration_and_queues_exactly_once_after_collection() {
    let f=Fixture::new(); let bid=f.book("单本书货源流程");
    let program=f.path.join("model-fixture"); fs::write(&program,[]).unwrap();
    { let c=lock(&f.state); db::set_setting(&c,"agentOpenCode",&json!({"mode":"local","path":program})).unwrap(); }
    let call=|body:Value|price_search::control(State(f.state.clone()),Path(bid),Json(body));
    let first=call(json!({"action":"start","ai":{"provider":"opencode","model":"fixture/source-model","prompt":"只比较单本书货源"}})).await.unwrap().0;
    let request_id=first["requestId"].clone();
    let item=json!({"url":"https://item.taobao.com/item.htm?id=456","title":"同版本候选","price":12});
    let _ = call(json!({"action":"batch","requestId":request_id,"batch":1,"items":[item]})).await.unwrap();
    assert_eq!(lock(&f.state).query_row("SELECT count(*) FROM collection_jobs WHERE kind='ai'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    // The staged request survives a restart and does not use newly saved defaults.
    let restarted=AppState::new(f.path.clone(),0).unwrap(); drop(restarted);
    let _ = call(json!({"action":"resume","requestId":request_id})).await.unwrap();
    let done=call(json!({"action":"finish","requestId":request_id,"reason":"visible-end"})).await.unwrap().0;
    assert!(done["analysisJobId"].as_i64().is_some());
    let duplicate=call(json!({"action":"finish","requestId":request_id,"reason":"visible-end"})).await.unwrap().0;
    assert_eq!(done["analysisJobId"],duplicate["analysisJobId"]);
    let c=lock(&f.state);
    let req:String=c.query_row("SELECT request FROM collection_jobs WHERE id=?",[done["analysisJobId"].as_i64().unwrap()],|r|r.get(0)).unwrap();
    let req:Value=serde_json::from_str(&req).unwrap();
    assert_eq!(req["task"],"sources"); assert_eq!(req["model"],"fixture/source-model"); assert_eq!(req["prompt"],"只比较单本书货源");
    assert_eq!(c.query_row("SELECT count(*) FROM collection_jobs WHERE kind='ai'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT count(*) FROM book_sources WHERE is_primary=1",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[tokio::test]
async fn manual_shipping_needs_no_purchase_and_preserves_local_tracking_across_sync() {
    let f=Fixture::new();let o=order("manual-ship","unmapped");f.observe(&o);
    let id=f.orders()[0]["id"].as_i64().unwrap();
    let mut input=json!({"trackingNo":"YT12345678","expressCode":"YTO","expressName":"圆通","confirmed":true});
    assert!(fulfillment::tracking(State(f.state.clone()),Path(id),Json(input.clone())).await.is_err());
    assert_eq!(f.orders()[0]["kind"],"unknown");
    input["physicalConfirmed"]=json!(true);
    let _ = fulfillment::tracking(State(f.state.clone()),Path(id),Json(input.clone())).await.unwrap();
    assert!(fulfillment::purchase_list(&lock(&f.state),id).unwrap().is_empty());
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"],"ready_to_ship");
    assert_eq!(f.orders()[0]["snapshot"]["order_status"],12);
    assert_eq!(f.orders()[0]["data"]["shipping"]["trackingNo"],"YT12345678");
    {
        let operation = f.state.xianyu.operation.lock().await;
        assert!(fulfillment::tracking(State(f.state.clone()),Path(id),Json(input.clone())).await.is_err());
        drop(operation);
    }
    lock(&f.state).execute("UPDATE fulfillment_orders SET data=json_set(data,'$.shippingUncertain',json('true')) WHERE id=?",[id]).unwrap();
    assert!(fulfillment::tracking(State(f.state.clone()),Path(id),Json(input.clone())).await.is_err());
    let bid=f.book("电子用书");f.channel(bid,"mixed");f.observe(&order("digital-only","9007199254741002"));
    let digital=f.orders().into_iter().find(|v|v["orderNo"]=="digital-only").unwrap()["id"].as_i64().unwrap();
    assert!(fulfillment::tracking(State(f.state.clone()),Path(digital),Json(input)).await.is_err());
}

#[tokio::test]
async fn manually_linked_taobao_orders_are_internal_unique_and_keep_unknown_cost() {
    let f=Fixture::new();let o=order("local-link","unmapped");f.observe(&o);f.observe(&order("other-sale","unmapped"));
    let records=f.orders();let id=records.iter().find(|v|v["orderNo"]=="local-link").unwrap()["id"].as_i64().unwrap();
    let other=records.iter().find(|v|v["orderNo"]=="other-sale").unwrap()["id"].as_i64().unwrap();
    let mut input=json!({"platform":"taobao","purchaseNo":"TB887766","state":"ordered","actualCost":null,"url":"https://item.taobao.com/item.htm?id=12","shop":"内部供货店","sku":"第2版","note":"内部备注","confirmed":true,"physicalConfirmed":true});
    let _ = fulfillment::purchase(State(f.state.clone()),Path(id),Json(input.clone())).await.unwrap();
    let _ = fulfillment::purchase(State(f.state.clone()),Path(id),Json(input.clone())).await.unwrap();
    assert_eq!(fulfillment::purchase_list(&lock(&f.state),id).unwrap().len(),1);
    assert!(fulfillment::purchase(State(f.state.clone()),Path(other),Json(input.clone())).await.is_err());
    input["state"]=json!("shipped");input["trackingNo"]=json!("SF8899");input["expressCode"]=json!("SF");input["expressName"]=json!("顺丰");
    let _ = fulfillment::purchase(State(f.state.clone()),Path(id),Json(input.clone())).await.unwrap();f.observe(&o);
    let details=fulfillment::detail(State(f.state.clone()),Path(id)).await.unwrap().0;
    assert_eq!(details["state"],"ready_to_ship");assert_eq!(details["snapshot"]["order_status"],12);
    assert_eq!(details["data"]["shipping"]["origin"],"purchase");assert_eq!(details["purchases"][0]["data"]["purchaseNo"],"TB887766");
    assert!(details["purchases"][0]["data"]["actualCost"].is_null());
    let restarted=AppState::new(f.path.clone(),0).unwrap();
    let persisted=fulfillment::detail(State(restarted.clone()),Path(id)).await.unwrap().0;
    assert_eq!(persisted["state"],"ready_to_ship");assert_eq!(persisted["purchases"][0]["data"]["purchaseNo"],"TB887766");drop(restarted);
    input["purchaseNo"]=json!("different");assert!(fulfillment::purchase(State(f.state.clone()),Path(id),Json(input.clone())).await.is_err());
    input["purchaseNo"]=json!("TB887766");input["state"]=json!("canceled");input["supplierRefund"]=json!(0);
    let _ = fulfillment::purchase(State(f.state.clone()),Path(id),Json(input)).await.unwrap();
    let details=fulfillment::detail(State(f.state.clone()),Path(id)).await.unwrap().0;
    assert!(details["data"]["shipping"].is_null());assert!(details["purchases"][0]["data"]["actualCost"].is_null());
}

#[tokio::test]
async fn lowest_price_scroll_batches_are_durable_deduplicated_and_cancelable() {
    let f = Fixture::new(); let bid = f.book("数学分析");
    let call = |body:Value| price_search::control(State(f.state.clone()),Path(bid),Json(body));
    let first = call(json!({"action":"start"})).await.unwrap().0;
    let req = first["requestId"].clone();
    assert_eq!(first["query"],"9787300000001");
    assert_eq!(reqwest::Url::parse(first["searchUrl"].as_str().unwrap()).unwrap().query_pairs().find(|(k,_)|k=="q").unwrap().1,"9787300000001");
    assert!(first["searchUrl"].as_str().unwrap().contains("sort=price-asc"));
    let item = |id:i64,price:f64| json!({"url":format!("https://item.taobao.com/item.htm?id={id}"),"title":"候选版本","shop":"教材店","price":price});
    let p1 = call(json!({"action":"page","requestId":req,"page":1,"items":[item(1,0.5),item(1,0.5),item(2,10.0)]})).await.unwrap().0;
    assert_eq!(p1["count"],2); assert_eq!(p1["pagesDone"],1);
    assert!(!p1["searchUrl"].as_str().unwrap().contains("s=44"));
    assert!(!p1["searchUrl"].as_str().unwrap().contains("taoshuPricePage"));
    assert_eq!(p1["mode"],"scroll"); assert_eq!(p1["batchesDone"],1); assert_eq!(p1["maxItems"],180);
    assert!(p1["bestVerified"].is_null()); assert_eq!(p1["bestDisplay"]["displayPrice"],0.5);
    let repeated = call(json!({"action":"page","requestId":req,"page":2,"items":[item(1,0.5)]})).await.unwrap().0;
    assert_eq!(repeated["state"],"waiting"); assert_eq!(repeated["pagesDone"],1);
    let _ = call(json!({"action":"resume","requestId":req})).await.unwrap();
    let _ = call(json!({"action":"page","requestId":req,"page":2,"items":[item(2,10.0),item(3,11.0)]})).await.unwrap();
    let complete = call(json!({"action":"page","requestId":req,"page":3,"items":[item(4,20.0)]})).await.unwrap().0;
    assert_eq!(complete["state"],"completed"); assert_eq!(complete["count"],4);
    assert_eq!(call(json!({"action":"page","requestId":req,"page":3,"items":[item(4,20.0)]})).await.unwrap().0["count"],4);
    let new = call(json!({"action":"start"})).await.unwrap().0;
    assert!(call(json!({"action":"page","requestId":req,"page":1,"items":[item(99,1.0)]})).await.is_err());
    let restarted = AppState::new(f.path.clone(),0).unwrap();
    assert_eq!(price_search::status(State(restarted.clone()),Path(bid)).await.unwrap().0["state"],"waiting");
    drop(restarted);
    let _ = call(json!({"action":"resume","requestId":new["requestId"]})).await.unwrap();
    let other = f.book("另一本书");
    let _ = price_search::control(State(f.state.clone()),Path(other),Json(json!({"action":"start"}))).await.unwrap();
    assert_eq!(price_search::status(State(f.state.clone()),Path(bid)).await.unwrap().0["state"],"waiting");
    assert!(call(json!({"action":"page","requestId":new["requestId"],"page":1,"items":[item(99,1.0)]})).await.is_err());
    let _ = call(json!({"action":"resume","requestId":new["requestId"]})).await.unwrap();
    assert_eq!(price_search::status(State(f.state.clone()),Path(other)).await.unwrap().0["state"],"waiting");
    assert_eq!(call(json!({"action":"cancel","requestId":new["requestId"]})).await.unwrap().0["state"],"canceled");
    assert!(call(json!({"action":"page","requestId":new["requestId"],"page":1,"items":[item(99,1.0)]})).await.is_err());
    assert_eq!(lock(&f.state).query_row("SELECT COUNT(*) FROM books",[],|r|r.get::<_,i64>(0)).unwrap(),2);
}

#[tokio::test]
async fn scroll_completion_keeps_price_labels_rejects_empty_and_preserves_old_tasks() {
    let f = Fixture::new(); let bid = f.book("滚动测试");
    let call = |body:Value| price_search::control(State(f.state.clone()),Path(bid),Json(body));
    let first = call(json!({"action":"start"})).await.unwrap().0;
    let req = first["requestId"].clone();
    assert!(call(json!({"action":"finish","requestId":req,"reason":"visible-end"})).await.is_err());
    assert!(call(json!({"action":"batch","requestId":req,"batch":2,"items":[]})).await.is_err());
    let batch = call(json!({"action":"batch","requestId":req,"batch":1,"items":[{"url":"https://item.taobao.com/item.htm?id=551","title":"补贴候选","price":52.48,"priceLabel":"补贴后"}]})).await.unwrap().0;
    assert_eq!(batch["count"],1); assert_eq!(batch["batchesDone"],1);
    assert_eq!(batch["rows"][0]["priceLabel"],"补贴后");
    assert_eq!(batch["bestDisplay"]["displayPrice"],52.48); assert!(batch["bestVerified"].is_null());
    let replay = call(json!({"action":"batch","requestId":req,"batch":1,"items":[]})).await.unwrap().0;
    assert_eq!(replay["count"],1);
    let _ = call(json!({"action":"waiting","requestId":req,"error":"模拟暂停"})).await.unwrap();
    assert!(call(json!({"action":"finish","requestId":req,"reason":"visible-end"})).await.is_err());
    let _ = call(json!({"action":"resume","requestId":req})).await.unwrap();
    let done = call(json!({"action":"finish","requestId":req,"reason":"visible-end"})).await.unwrap().0;
    assert_eq!(done["state"],"completed"); assert_eq!(done["completionReason"],"visible-end");
    assert_eq!(call(json!({"action":"finish","requestId":req,"reason":"visible-end"})).await.unwrap().0["count"],1);
    // Existing snapshots keep their counters and candidates, but resume from the real page.
    lock(&f.state).execute("UPDATE price_searches SET data=? WHERE book_id=?",params![json!({"requestId":"old-task","query":"教材","state":"waiting","pagesDone":1,"maxPages":3,"items":batch["items"]}).to_string(),bid]).unwrap();
    let legacy = price_search::status(State(f.state.clone()),Path(bid)).await.unwrap().0;
    assert_eq!(legacy["batchesDone"],1);assert_eq!(legacy["count"],1);
    assert!(!legacy["searchUrl"].as_str().unwrap().contains("taoshuPricePage"));
}

#[tokio::test]
async fn lowest_verified_price_includes_freight_condition_and_dropship_without_overwriting_quotes() {
    let f = Fixture::new(); let bid = f.book("数学分析");
    let condition = db::get_book(&lock(&f.state),bid).unwrap().unwrap().condition;
    let started = price_search::control(State(f.state.clone()),Path(bid),Json(json!({"action":"start"}))).await.unwrap().0;
    for (id,price,freight,dropship,kind) in [(11,20.0,Some(3.0),true,condition.clone()),(12,10.0,Some(20.0),true,condition.clone()),(13,0.1,None,true,condition.clone()),(14,0.1,Some(0.0),false,condition.clone()),(15,0.1,Some(0.0),true,"不同品相".into())] {
        let _ = save_source(State(f.state.clone()),Json(json!({"bookId":bid,"data":{"url":format!("https://item.taobao.com/item.htm?id={id}"),"title":"核验货源","shop":"测试店","sku":"已核对版次册次","condition":kind,"status":"available","checkedAt":"2026-10-27","dropship":dropship,"price":price,"freight":freight}}))).await.unwrap();
    }
    let v = price_search::status(State(f.state.clone()),Path(bid)).await.unwrap().0;
    assert_eq!(v["bestVerified"]["total"],23.0);
    assert_eq!(v["rows"].as_array().unwrap().iter().filter(|r|!r["total"].is_null()).count(),2);
    let _ = price_search::control(State(f.state.clone()),Path(bid),Json(json!({"action":"page","requestId":started["requestId"],"page":1,"items":[{"url":"https://item.taobao.com/item.htm?id=11","title":"最低规格起价","price":0.01}]}))).await.unwrap();
    let after = price_search::status(State(f.state.clone()),Path(bid)).await.unwrap().0;
    assert_eq!(after["bestDisplay"]["displayPrice"],0.01);
    assert_eq!(after["bestVerified"]["total"],23.0);
    assert_eq!(after["bestVerified"]["source"]["data"]["price"],20.0);
    lock(&f.state).execute("UPDATE books SET condition='' WHERE id=?",[bid]).unwrap();
    assert!(price_search::status(State(f.state.clone()),Path(bid)).await.unwrap().0["bestVerified"].is_null());
}

#[test]
fn taobao_search_candidates_are_book_scoped_idempotent_and_never_move_reviewed_sources() {
    let f = Fixture::new();
    let a = f.book("第5版");
    let b = f.book("第6版");
    let input: CaptureInput = serde_json::from_value(json!({"asSource":true,"sourceBookId":a,"itemId":"1234","title":"多版本商品","price":12,"url":"https://item.taobao.com/item.htm?id=1234"})).unwrap();
    let c = lock(&f.state);
    let (capture,book,created) = db::upsert_capture(&c,&input,0.3,5.0,"九成新").unwrap();
    assert!(book.is_none() && !created);
    capture_candidate(&c,&capture).unwrap();
    attach_capture_candidate(&c,&capture,a).unwrap();
    let source_a = source_list(&c,Some(a)).unwrap()[0].clone();
    assert_eq!(source_a["data"]["status"],"pending");
    assert_eq!(source_a["data"]["dropship"],false);
    c.execute("UPDATE book_sources SET data=json_set(data,'$.sku','第5版','$.price',10,'$.status','available') WHERE book_id=?",[a]).unwrap();
    capture_candidate(&c,&capture).unwrap();
    attach_capture_candidate(&c,&capture,b).unwrap();
    attach_capture_candidate(&c,&capture,b).unwrap();
    assert_eq!(source_list(&c,Some(a)).unwrap()[0]["data"]["price"],10);
    assert_eq!(source_list(&c,Some(a)).unwrap()[0]["data"]["sku"],"第5版");
    assert_eq!(source_list(&c,Some(b)).unwrap().len(),1);
    assert_eq!(source_list(&c,Some(b)).unwrap()[0]["data"]["status"],"pending");
    assert_eq!(c.query_row("SELECT COUNT(*) FROM books",[],|r|r.get::<_,i64>(0)).unwrap(),2);
}

#[tokio::test]
async fn online_book_search_keeps_evidence_and_rejects_expired_results() {
    let f = Fixture::new();
    lock(&f.state).execute("INSERT INTO school_requirements(id,school,year,major,data,updated_at) VALUES('online-test','学校',2027,'专业',?,'now')",
        [json!({"raw":"数据库 第5版","candidate":{"title":"数据库","authors":["王珊"],"edition":"第5版"}}).to_string()]).unwrap();
    let first = book_search::update(State(f.state.clone()),Path("online-test".into()),Json(json!({"action":"start"}))).await.unwrap().0;
    assert_eq!(first["query"],"数据库 王珊 第5版");
    assert!(first["searchUrl"].as_str().unwrap().starts_with("https://search.kongfz.com/product/"));
    let second = book_search::update(State(f.state.clone()),Path("online-test".into()),Json(json!({"action":"start","query":"9787040406641"}))).await.unwrap().0;
    assert!(book_search::update(State(f.state.clone()),Path("online-test".into()),Json(json!({"requestId":first["requestId"],"items":[]}))).await.is_err());
    let result = book_search::update(State(f.state.clone()),Path("online-test".into()),Json(json!({"requestId":second["requestId"],"items":[{"title":"数据库系统概论","edition":"第5版","isbn":"9787040406641","sourceUrl":"https://book.kongfz.com/123/456/"}]}))).await.unwrap().0;
    assert_eq!(result["state"],"completed");
    assert_eq!(result["items"][0]["isbn"],"9787040406641");
    assert_eq!(book_search::status(State(f.state.clone()),Path("online-test".into())).await.unwrap().0,result);
    assert_eq!(lock(&f.state).query_row("SELECT COUNT(*) FROM books",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

struct Fixture {
    state: AppState,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "taoshu-commerce-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let state = AppState::new(path.clone(), 0).unwrap();
        db::set_setting(&lock(&state), "xianyuAppKey", &json!("test-app")).unwrap();
        Self { state, path }
    }
    fn book(&self, title: &str) -> i64 {
        db::create_book(
            &lock(&self.state),
            &serde_json::from_value::<BookPatch>(
                json!({"title":title,"isbn":"9787300000001","edition":"第2版","stock":7}),
            )
            .unwrap(),
        )
        .unwrap()
        .id
    }
    fn channel(&self, bid: i64, mode: &str) {
        lock(&self.state).execute("INSERT INTO channel_products(book_id,app_key,shop,product_id,data,updated_at) VALUES(?,'test-app','shop','9007199254740997',?,'now')",params![bid,json!({"mode":mode,"title":"教材","itemId":"8007199254740997","physicalSkuId":"9007199254741001","digitalSkuId":"9007199254741002","state":"published"}).to_string()]).unwrap();
    }
    fn observe(&self, order: &Value) {
        observe_order(&lock(&self.state), "test-app", order).unwrap();
    }
    fn orders(&self) -> Vec<Value> {
        fulfillment::all(&lock(&self.state)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only the unique test directory is cleaned; SQLite handles may still be held on Windows.
        let _ = lock(&self.state).execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        let old = std::mem::replace(
            &mut *lock(&self.state),
            Connection::open_in_memory().unwrap(),
        );
        drop(old);
        let _ = fs::remove_dir_all(&self.path);
    }
}
fn order(no: &str, sku: &str) -> Value {
    json!({"order_no":no,"seller_name":"shop","order_status":12,"refund_status":0,"refund_amount":0,"refund_time":0,"pay_amount":5000,"update_time":100,"goods":{"title":"教材","quantity":1,"product_id":9007199254740997u64,"item_id":8007199254740997u64,"sku_id":sku}})
}
fn catalog() -> Value {
    json!({"schools":[{"id":"s","name":"甲大学"},{"id":"s2","name":"乙大学"}],"sources":[{"id":"source","collectedAt":"2026-10-20"}],"programs":[{"id":"p","schoolId":"s","sourceId":"source","year":2027,"college":"数学学院","majorName":"数学","majorCode":"0701"},{"id":"p2","schoolId":"s2","sourceId":"source","year":2027,"majorName":"数学"}],"subjects":[{"id":"sub","programId":"p","stage":"初试","name":"数学分析"},{"id":"sub2","programId":"p2","stage":"初试","name":"数学分析"}],"books":[{"id":"b","title":"数学分析","editions":["第2版","第3版"],"volumes":["上册","下册"]}],"requirements":[{"id":"r","subjectId":"sub","textbookId":"b","raw":"数学分析（第2版）上册","sourceUrl":"https://example.org/catalog"},{"id":"r2","subjectId":"sub2","textbookId":"b","raw":"数学分析（第2版）上册"}]})
}
async fn source(f: &Fixture, bid: i64, primary: bool, price: f64) -> i64 {
    save_source(State(f.state.clone()),Json(json!({"bookId":bid,"primary":primary,"data":{"url":"https://item.taobao.com/item.htm?id=123","shop":"供货店","sku":"第2版 上册","status":"available","dropship":true,"price":price,"freight":2,"checkedAt":"2026-10-23T01:00:00Z"}}))).await.unwrap().0["id"].as_i64().unwrap()
}

#[tokio::test]
async fn school_preview_repeat_and_multi_school_confirmation() {
    let f = Fixture::new();
    let bid = f.book("数学分析");
    let input = catalog();
    let p = catalog::preview(State(f.state.clone()), Json(input.clone()))
        .await
        .unwrap()
        .0;
    assert_eq!(p["count"], 2);
    assert_eq!(p["records"][0]["ambiguous"], true);
    assert!(
        catalog::list(State(f.state.clone())).await.unwrap().0["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let _ = catalog::import(State(f.state.clone()), Json(input.clone()))
        .await
        .unwrap();
    for r in p["records"].as_array().unwrap() {
        let _ = catalog::confirm(
            State(f.state.clone()),
            Path(text(r, "id").into()),
            Json(json!({"bookId":bid,"editionChecked":true})),
        )
        .await
        .unwrap();
    }
    let mut newer = input.clone();
    newer["sources"][0]["collectedAt"] = json!("2026-10-23");
    assert_eq!(
        catalog::import(State(f.state.clone()), Json(newer))
            .await
            .unwrap()
            .0["unchanged"],
        2
    );
    let result = catalog::list(State(f.state.clone())).await.unwrap().0;
    assert!(result["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["bookId"] == bid && r["state"] == "confirmed"));
    assert_eq!(
        books(State(f.state.clone()))
            .await
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut changed = input;
    changed["requirements"][0]["raw"] = json!("数学分析第3版 下册");
    assert_eq!(
        catalog::import(State(f.state.clone()), Json(changed))
            .await
            .unwrap()
            .0["changed"],
        1
    );
    assert_eq!(
        catalog::list(State(f.state.clone())).await.unwrap().0["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["state"] == "pending")
            .count(),
        1
    );
}
#[tokio::test]
async fn different_editions_are_not_automatically_merged() {
    let f = Fixture::new();
    let input = catalog();
    let records = catalog::flatten(&input).unwrap();
    let _ = catalog::import(State(f.state.clone()), Json(input))
        .await
        .unwrap();
    for (i, r) in records.iter().enumerate() {
        let _ = catalog::confirm(State(f.state.clone()),Path(text(r,"id").into()),Json(json!({"editionChecked":true,"book":{"title":"数学分析","edition":format!("第{}版",i+2),"volume":"上册"}}))).await.unwrap();
    }
    assert_eq!(
        books(State(f.state.clone()))
            .await
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(catalog::confirm(
        State(f.state.clone()),
        Path(text(&records[0], "id").into()),
        Json(json!({"bookId":1}))
    )
    .await
    .is_err());
}
#[tokio::test]
async fn candidates_primary_sources_and_immutable_resource_versions() {
    let f = Fixture::new();
    let input:CaptureInput=serde_json::from_value(json!({"itemId":"123","url":"https://item.taobao.com/item.htm?id=123","title":"候选","asSource":true})).unwrap();
    let (capture, book, _) =
        db::upsert_capture(&lock(&f.state), &input, 0.3, 5.0, "九成新").unwrap();
    assert!(book.is_none());
    capture_candidate(&lock(&f.state), &capture).unwrap();
    capture_candidate(&lock(&f.state), &capture).unwrap();
    assert_eq!(
        books(State(f.state.clone()))
            .await
            .unwrap()
            .0
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(source_list(&lock(&f.state), None).unwrap().len(), 1);
    let bid = f.book("数学分析");
    source(&f, bid, true, 10.0).await;
    source(&f, bid, true, 12.0).await;
    assert_eq!(
        source_list(&lock(&f.state), Some(bid))
            .unwrap()
            .iter()
            .filter(|r| r["primary"] == true)
            .count(),
        1
    );
    let payload = json!({"version":"v1","url":"https://example.org/drive","instructions":"打开链接下载","code":"abcd"});
    let id = add_resource(State(f.state.clone()), Path(bid), Json(payload.clone()))
        .await
        .unwrap()
        .0["id"]
        .as_i64()
        .unwrap();
    assert!(
        add_resource(State(f.state.clone()), Path(bid), Json(payload))
            .await
            .is_err()
    );
    assert!(resource_state(
        State(f.state.clone()),
        Path(id),
        Json(json!({"state":"ready"}))
    )
    .await
    .is_err());
    let _ = resource_state(
        State(f.state.clone()),
        Path(id),
        Json(json!({"state":"ready","checked":true})),
    )
    .await
    .unwrap();
    let c = lock(&f.state);
    assert_eq!(
        summary(&c, &db::get_book(&c, bid).unwrap().unwrap()).unwrap()["digitalReady"],
        true
    );
}
#[test]
fn exact_mapping_app_shop_spec_and_string_identifiers() {
    let f = Fixture::new();
    let bid = f.book("数学分析");
    f.channel(bid, "mixed");
    let mut o = order("1", "9007199254741001");
    f.observe(&o);
    o["order_no"] = json!("2");
    o["seller_name"] = json!("wrong-shop");
    o["goods"]["item_id"] = json!("wrong-item");
    f.observe(&o);
    o = order("3", "");
    f.observe(&o);
    o = order("4", "0");
    f.observe(&o);
    observe_order(
        &lock(&f.state),
        "another-app",
        &order("5", "9007199254741001"),
    )
    .unwrap();
    let all = f.orders();
    assert_eq!(all.len(), 4);
    assert_eq!(all.iter().filter(|o| o["kind"] == "physical").count(), 1);
    let known = all.iter().find(|o| o["orderNo"] == "1").unwrap();
    assert_eq!(known["snapshot"]["goods"]["product_id"], "9007199254740997");
    assert!(all
        .iter()
        .filter(|o| o["kind"] == "unknown")
        .all(|o| o["bookId"].is_null()));
}
#[tokio::test]
async fn physical_repeat_sync_purchase_replacement_and_refund() {
    let f = Fixture::new();
    let bid = f.book("数学分析");
    let first = source(&f, bid, true, 10.0).await;
    f.channel(bid, "mixed");
    let mut o = order("physical", "9007199254741001");
    f.observe(&o);
    f.observe(&o);
    assert_eq!(f.orders().len(), 1);
    let id = f.orders()[0]["id"].as_i64().unwrap();
    let backup = source(&f, bid, true, 20.0).await;
    f.observe(&o);
    assert_eq!(f.orders()[0]["data"]["source"]["id"], first);
    let payload = json!({"sourceId":first,"state":"ordered","purchaseNo":"tb001","actualCost":12,"confirmed":true});
    let _ = fulfillment::purchase(State(f.state.clone()), Path(id), Json(payload.clone()))
        .await
        .unwrap();
    let _ = fulfillment::purchase(State(f.state.clone()), Path(id), Json(payload))
        .await
        .unwrap();
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "awaiting_tracking");
    assert_eq!(
        fulfillment::purchase_list(&lock(&f.state), id)
            .unwrap()
            .len(),
        1
    );
    assert!(fulfillment::purchase(
        State(f.state.clone()),
        Path(id),
        Json(json!({"sourceId":backup,"state":"draft"}))
    )
    .await
    .is_err());
    o["refund_status"] = json!(1);
    o["update_time"] = json!(101);
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "after_sale");
    assert!(fulfillment::purchase(State(f.state.clone()),Path(id),Json(json!({"sourceId":first,"state":"ordered","confirmed":true,"purchaseNo":"tb002","actualCost":12}))).await.is_err());
    let _ = fulfillment::purchase(
        State(f.state.clone()),
        Path(id),
        Json(
            json!({"sourceId":first,"state":"canceled","confirmed":true,"supplierRefund":12,"note":"已在淘宝完成退款"}),
        ),
    )
    .await
    .unwrap();
    let _ = fulfillment::note(
        State(f.state.clone()),
        Path(id),
        Json(json!({"note":"销售与采购已核查","close":true})),
    )
    .await
    .unwrap();
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "closed");
    o["refund_status"] = json!(5);
    o["refund_amount"] = json!(5000);
    o["update_time"] = json!(102);
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "after_sale");
    o["refund_status"] = json!(4);
    o["update_time"] = json!(103);
    f.observe(&o);
    let _ = fulfillment::purchase(
        State(f.state.clone()),
        Path(id),
        Json(json!({"sourceId":backup,"state":"draft"})),
    )
    .await
    .unwrap();
    assert_eq!(
        fulfillment::purchase_list(&lock(&f.state), id)
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
async fn unknown_shipping_survives_sync_and_outdated_orders_are_ignored() {
    let f = Fixture::new();
    let bid = f.book("书");
    f.channel(bid, "mixed");
    let mut o = order("ship", "9007199254741001");
    f.observe(&o);
    lock(&f.state)
        .execute("UPDATE fulfillment_orders SET state='shipping_unknown'", [])
        .unwrap();
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "shipping_unknown");
    o["update_time"] = json!(101);
    o["order_status"] = json!(21);
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "shipped");
    o["update_time"] = json!(99);
    o["order_status"] = json!(12);
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "shipped");
}
#[tokio::test]
async fn digital_delivery_requires_evidence_and_preserves_resource_snapshot() {
    let f = Fixture::new();
    let bid = f.book("书");
    f.channel(bid, "mixed");
    let r=add_resource(State(f.state.clone()),Path(bid),Json(json!({"version":"v1","url":"https://example.org/one","instructions":"下载","code":"1234"}))).await.unwrap().0["id"].as_i64().unwrap();
    lock(&f.state)
        .execute(
            "UPDATE channel_products SET data=json_set(data,'$.resourceId',?)",
            [r],
        )
        .unwrap();
    let mut o = order("digital", "9007199254741002");
    o["goods"]["quantity"] = json!(2);
    f.observe(&o);
    lock(&f.state)
        .execute(
            "UPDATE channel_products SET data=json_remove(data,'$.resourceId')",
            [],
        )
        .unwrap();
    f.observe(&o);
    assert_eq!(f.orders()[0]["data"]["resource"]["version"], "v1");
    for cards in [
        json!({"list":[]}),
        json!({"list":[{"sold_type":21,"card_no":"one"}]}),
        json!({"list":[{"sold_type":11,"card_no":"one"}]}),
    ] {
        record_delivery(&lock(&f.state), "test-app", "digital", &cards).unwrap();
        assert_eq!(f.orders()[0]["state"], "delivery_pending");
    }
    let cards = json!({"list":[{"sold_type":11,"card_no":"one"},{"sold_type":11,"card_no":"two"}]});
    record_delivery(&lock(&f.state), "test-app", "digital", &cards).unwrap();
    record_delivery(&lock(&f.state), "test-app", "digital", &cards).unwrap();
    assert_eq!(f.orders()[0]["state"], "delivered");
    assert_eq!(
        lock(&f.state)
            .query_row("SELECT count(*) FROM delivery_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db::get_book(&lock(&f.state), bid).unwrap().unwrap().stock,
        7
    );
    o["order_status"] = json!(24);
    o["update_time"] = json!(101);
    f.observe(&o);
    assert_eq!(f.orders()[0]["state"], "canceled_review");
}
#[tokio::test]
async fn one_channel_per_book_shop_and_mixed_publish_gate() {
    let f = Fixture::new();
    let bid = f.book("书");
    source(&f, bid, true, 10.0).await;
    let payload = json!({"bookId":bid,"shop":"shop","data":{"mode":"mixed","title":"书","physicalPrice":20,"digitalPrice":5}});
    let ch = save_channel(State(f.state.clone()), Json(payload.clone()))
        .await
        .unwrap()
        .0;
    let _ = save_channel(State(f.state.clone()), Json(payload.clone()))
        .await
        .unwrap();
    let check = check_channel(&lock(&f.state), ch["id"].as_i64().unwrap()).unwrap();
    assert_eq!(check["ready"], false);
    assert_eq!(capability()["canPublishMixed"], false);
    assert_eq!(channel_list(&lock(&f.state), bid).unwrap().len(), 1);
    let mut batch = payload;
    batch["createOnly"] = json!(true);
    assert!(save_channel(State(f.state.clone()), Json(batch))
        .await
        .is_err());
}
#[test]
fn backup_and_idempotent_migration_preserve_old_book_and_capture() {
    let f = Fixture::new();
    let input:CaptureInput=serde_json::from_value(json!({"itemId":"legacy","title":"旧书","url":"https://item.taobao.com/item.htm?id=1","price":12})).unwrap();
    let (_, book, _) = db::upsert_capture(&lock(&f.state), &input, 0.3, 5.0, "九成新").unwrap();
    let bid = book.unwrap().id;
    {
        let c = lock(&f.state);
        c.execute("DROP TABLE commerce_meta", []).unwrap();
        schema::backup(&c, &f.path).unwrap();
        schema::init(&c).unwrap();
        schema::init(&c).unwrap();
        assert_eq!(source_list(&c, Some(bid)).unwrap().len(), 1);
        assert_eq!(db::get_book(&c, bid).unwrap().unwrap().title, "旧书");
    }
    let backups = fs::read_dir(&f.path)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("shelf-before-commerce")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(backups[0].path()).unwrap();
    assert_eq!(
        backup
            .query_row("SELECT title FROM books WHERE id=?", [bid], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "旧书"
    );
}

#[tokio::test]
async fn invalid_resources_create_tasks_and_finance_never_assumes_zero_cost() {
    let f = Fixture::new();
    let bid = f.book("书");
    let sid = source(&f, bid, true, 10.0).await;
    f.channel(bid, "mixed");
    f.observe(&order("finance", "9007199254741001"));
    let id = f.orders()[0]["id"].as_i64().unwrap();
    let m = overview(State(f.state.clone())).await.unwrap().0;
    assert_eq!(m["metrics"]["physical"]["revenue"], 5000);
    assert_eq!(m["metrics"]["physical"]["pendingCost"], 1);
    assert_eq!(m["metrics"]["physical"]["margin"], 0);
    let _=fulfillment::purchase(State(f.state.clone()),Path(id),Json(json!({"sourceId":sid,"state":"ordered","actualCost":12,"purchaseNo":"tb-finance","confirmed":true}))).await.unwrap();
    assert!(fulfillment::purchase(
        State(f.state.clone()),
        Path(id),
        Json(json!({"sourceId":sid,"state":"draft"}))
    )
    .await
    .is_err());
    let _ = fulfillment::finance(
        State(f.state.clone()),
        Path(id),
        Json(json!({"fees":2,"refund":5})),
    )
    .await
    .unwrap();
    let m = overview(State(f.state.clone())).await.unwrap().0;
    assert_eq!(m["metrics"]["physical"]["margin"], 3100);
    assert_eq!(m["metrics"]["physical"]["pendingCost"], 0);
    let _=fulfillment::purchase(State(f.state.clone()),Path(id),Json(json!({"sourceId":sid,"state":"canceled","supplierRefund":10,"note":"供应商扣除运费后退款","confirmed":true}))).await.unwrap();
    let m = overview(State(f.state.clone())).await.unwrap().0;
    assert_eq!(m["metrics"]["physical"]["cost"], 200);
    let res = add_resource(
        State(f.state.clone()),
        Path(bid),
        Json(json!({"version":"v1","url":"https://example.org/resource","instructions":"下载"})),
    )
    .await
    .unwrap()
    .0;
    let _ = resource_state(
        State(f.state.clone()),
        Path(res["id"].as_i64().unwrap()),
        Json(json!({"state":"invalid"})),
    )
    .await
    .unwrap();
    let m = overview(State(f.state.clone())).await.unwrap().0;
    assert!(m["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| text(t, "reason").contains("资源失效")));
}

#[tokio::test]
async fn library_filters_paginate_and_keep_zero_stock_dropship_books() {
    let f = Fixture::new();
    let bid = f.book("数学分析");
    let other = f.book("线性代数");
    lock(&f.state)
        .execute("UPDATE books SET stock=0 WHERE id=?", [bid])
        .unwrap();
    source(&f, bid, true, 15.0).await;
    let _ = catalog::import(State(f.state.clone()), Json(catalog()))
        .await
        .unwrap();
    let records = catalog::list(State(f.state.clone())).await.unwrap().0;
    let rid = text(
        records["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["data"]["school"] == "甲大学")
            .unwrap(),
        "id",
    );
    let _ = catalog::confirm(
        State(f.state.clone()),
        Path(rid.into()),
        Json(json!({"bookId":bid,"editionChecked":true})),
    )
    .await
    .unwrap();
    for (filter, total) in [
        (json!({"q":"数学分析"}), 1),
        (json!({"readiness":"sources"}), 1),
        (json!({"school":"甲大学"}), 1),
        (json!({"major":"无此专业"}), 0),
    ] {
        let response = library::list(
            State(f.state.clone()),
            axum::extract::Query(serde_json::from_value(filter).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(response["total"], total);
    }
    let page = library::list(
        State(f.state.clone()),
        axum::extract::Query(serde_json::from_value(json!({"pageSize":1,"page":99})).unwrap()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(page["page"], 2);
    assert_eq!(page["items"][0]["id"], bid);
    assert_eq!(page["items"][0]["stock"], 0);
    assert_eq!(page["items"][0]["physicalReady"], true);
    assert_ne!(other, bid);
}

#[tokio::test]
async fn catalog_candidate_identifier_migration_preserves_confirmed_citation() {
    let f = Fixture::new();
    let bid = f.book("数学分析");
    let old = catalog();
    let _ = catalog::import(State(f.state.clone()), Json(old.clone()))
        .await
        .unwrap();
    let rid = catalog::flatten(&old).unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let _ = catalog::confirm(
        State(f.state.clone()),
        Path(rid.clone()),
        Json(json!({"bookId":bid,"editionChecked":true})),
    )
    .await
    .unwrap();
    let mut next = old;
    next["requirements"][0]["id"] = json!("new-edition-key");
    next["books"][0]["edition"] = json!("第2版");
    let result = catalog::import(State(f.state.clone()), Json(next))
        .await
        .unwrap()
        .0;
    assert_eq!(result["added"], 0);
    assert_eq!(result["unchanged"], 2);
    let rows = catalog::list(State(f.state.clone())).await.unwrap().0;
    let row = rows["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == rid)
        .unwrap();
    assert_eq!(row["bookId"], bid);
    assert_eq!(row["state"], "confirmed");
    assert_eq!(rows["records"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn jobs_validate_controls_evidence_and_recover_after_restart() {
    let f = Fixture::new();
    let bid = f.book("数学分析");
    let job = jobs::create(
        State(f.state.clone()),
        Json(json!({"kind":"sources","bookIds":[bid,bid]})),
    )
    .await
    .unwrap()
    .0;
    let id = job["id"].as_i64().unwrap();
    assert_eq!(job["total"], 1);
    let control = |action: &str| Path((id, action.to_string()));
    assert!(jobs::control(
        State(f.state.clone()),
        control("complete-item"),
        Json(json!({"bookId":bid}))
    )
    .await
    .is_err());
    let _ = jobs::control(State(f.state.clone()), control("pause"), Json(json!({})))
        .await
        .unwrap();
    assert_eq!(
        jobs::detail(State(f.state.clone()), Path(id))
            .await
            .unwrap()
            .0["state"],
        "paused"
    );
    let _ = jobs::control(State(f.state.clone()), control("resume"), Json(json!({})))
        .await
        .unwrap();
    lock(&f.state)
        .execute(
            "UPDATE collection_jobs SET state='running' WHERE id=?",
            [id],
        )
        .unwrap();
    let recovered = AppState::new(f.path.clone(), 0).unwrap();
    assert_eq!(
        jobs::detail(State(recovered.clone()), Path(id))
            .await
            .unwrap()
            .0["state"],
        "queued"
    );
    lock(&f.state)
        .execute(
            "UPDATE collection_jobs SET state='waiting' WHERE id=?",
            [id],
        )
        .unwrap();
    assert!(jobs::control(
        State(f.state.clone()),
        control("complete-item"),
        Json(json!({"bookId":bid}))
    )
    .await
    .is_err());
    source(&f, bid, true, 15.0).await;
    let done = jobs::control(
        State(f.state.clone()),
        control("complete-item"),
        Json(json!({"bookId":bid})),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(done["state"], "completed");
    assert_eq!(done["processed"], 1);
    assert!(
        jobs::control(State(f.state.clone()), control("retry"), Json(json!({})))
            .await
            .is_err()
    );
    drop(recovered);
}
