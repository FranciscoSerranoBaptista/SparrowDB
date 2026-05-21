use crate::http::SparrowClient;

#[test]
fn client_builds_with_base_url() {
    let client = SparrowClient::new("http://localhost:6969");
    assert_eq!(client.base_url(), "http://localhost:6969");
}

#[test]
fn client_strips_trailing_slash() {
    let client = SparrowClient::new("http://localhost:6969/");
    assert_eq!(client.base_url(), "http://localhost:6969");
}
