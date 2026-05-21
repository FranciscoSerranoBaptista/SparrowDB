use crate::templates::{chef_prompt, docker_compose, queries_hx, read_json, schema_hx, seed_json};

#[test]
fn docker_compose_has_port_6969_and_volume() {
    let s = docker_compose();
    assert!(s.contains("6969"));
    assert!(s.contains("sparrow-data"));
    assert!(s.contains("SPARROW_DATA_DIR"));
}

#[test]
fn seed_json_is_valid_json_with_addn() {
    let s = seed_json();
    let v: serde_json::Value = serde_json::from_str(&s).expect("seed.json must be valid JSON");
    assert_eq!(v["request_type"], "write");
    let queries = &v["query"]["queries"];
    assert!(queries.is_array());
    assert!(
        queries[0].get("ForEach").is_some(),
        "first query must be ForEach"
    );
}

#[test]
fn read_json_is_valid_json_with_nwhere() {
    let s = read_json();
    let v: serde_json::Value = serde_json::from_str(&s).expect("read.json must be valid JSON");
    assert_eq!(v["request_type"], "read");
    let steps = &v["query"]["queries"][0]["Query"]["steps"];
    assert!(steps[0].get("NWhere").is_some());
}

#[test]
fn schema_hx_has_user_node() {
    let s = schema_hx();
    assert!(s.contains("N::User"));
}

#[test]
fn queries_hx_has_a_query() {
    let s = queries_hx();
    assert!(s.contains("QUERY"));
}

#[test]
fn chef_prompt_with_intent() {
    let s = chef_prompt("build a social network");
    assert!(s.contains("build a social network"));
    assert!(s.contains("localhost:6969"));
}

#[test]
fn chef_prompt_empty_intent_uses_default_spec() {
    let s = chef_prompt("");
    assert!(s.contains("Personal CRM"));
    assert!(s.contains("localhost:6969"));
}

#[test]
fn chef_prompt_whitespace_intent_uses_default_spec() {
    let s = chef_prompt("   ");
    assert!(s.contains("Personal CRM"));
}
