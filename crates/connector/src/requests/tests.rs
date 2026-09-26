use serde_json::{Value, json};

use super::*;

fn request(id: u128) -> Value {
    json!({
        "id": Uuid::from_u128(id),
        "requester": "alice",
        "reason": "ERP update",
        "minutes": 120,
        "targets": [{ "name": "sql", "host": "sql01.corp.local", "port": 1433 }],
    })
}

fn parsed(value: &Value) -> Result<Vec<CustomerRequest>, RequestsError> {
    parse(value.to_string().as_bytes())
}

/// Changes one field of the first request.
fn with(path: &[&str], value: Value) -> Value {
    let mut answer = json!({ "requests": [request(1)] });
    let mut place = &mut answer["requests"][0];
    for key in path {
        place = match key.parse::<usize>() {
            Ok(index) => &mut place[index],
            Err(_) => &mut place[*key],
        };
    }
    *place = value;
    answer
}

#[test]
fn a_well_formed_answer_is_taken() {
    let requests = parsed(&json!({ "requests": [request(1), request(2)] })).unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].targets[0].host, "sql01.corp.local");
    assert_eq!(parsed(&json!({ "requests": [] })).unwrap(), Vec::new());
}

#[test]
fn anything_unexpected_drops_the_whole_answer() {
    let cases = [
        // Fields remotehub never sends, e.g. something that looks like a command.
        json!({ "requests": [], "open": true }),
        with(&["approved"], json!(true)),
        with(&["targets", "0", "until"], json!("2099-01-01T00:00:00Z")),
        // Missing and mistyped fields.
        json!({}),
        with(&["minutes"], json!("120")),
        with(&["targets", "0", "port"], json!(70000)),
        // Out of bounds.
        with(&["minutes"], json!(14)),
        with(&["minutes"], json!(1441)),
        with(&["targets"], json!([])),
        with(&["targets", "0", "port"], json!(0)),
        with(&["reason"], json!("")),
        with(&["reason"], json!("x".repeat(501))),
        with(&["requester"], json!("a".repeat(201))),
        // Text that is not what it looks like.
        with(&["reason"], json!("fine\u{1b}[2J")),
        with(&["reason"], json!("line\nbreak")),
        with(&["targets", "0", "name"], json!("sql\u{202E}lmth.exe")),
        with(&["requester"], json!("ali\u{200B}ce")),
        // Hosts that are no host.
        with(&["targets", "0", "host"], json!("sql01 corp")),
        with(&["targets", "0", "host"], json!("-sql01")),
        with(&["targets", "0", "host"], json!("sql01.corp.local:22")),
        with(&["targets", "0", "host"], json!("http://sql01")),
        with(&["targets", "0", "host"], json!("")),
        with(
            &["targets", "0", "host"],
            json!(format!("{}.local", "a".repeat(64))),
        ),
    ];
    for case in cases {
        assert!(parsed(&case).is_err(), "taken: {case}");
    }
    // Structure that is fine in parts.
    assert_eq!(
        parsed(&json!({ "requests": [request(1), request(1)] })),
        Err(RequestsError::Twice(Uuid::from_u128(1)))
    );
    let many: Vec<Value> = (0..=MAX_REQUESTS as u128).map(request).collect();
    assert_eq!(
        parsed(&json!({ "requests": many })),
        Err(RequestsError::TooMany)
    );
    let targets: Vec<Value> = (0..=MAX_TARGETS)
        .map(|n| json!({ "name": format!("d{n}"), "host": "10.0.0.1", "port": 22 }))
        .collect();
    assert!(parsed(&with(&["targets"], json!(targets))).is_err());
    assert!(matches!(
        parse(b"not json"),
        Err(RequestsError::Malformed(_))
    ));
}

#[test]
fn an_oversized_answer_is_not_even_read() {
    let mut big = r#"{"requests":[],"padding":""#.to_owned();
    big.push_str(&" ".repeat(MAX_PENDING_BYTES));
    big.push_str("\"}");
    assert_eq!(parse(big.as_bytes()), Err(RequestsError::TooLong));
}

#[test]
fn addresses_of_both_kinds_are_hosts() {
    for good in ["10.0.0.5", "fe80::1", "sql01", "sql01.corp.local", "a-b.c"] {
        assert!(host(good), "{good}");
    }
}

#[test]
fn an_answer_goes_back_until_remotehub_stops_listing_the_request() {
    let requests = Requests::default();
    let first = parsed(&json!({ "requests": [request(1), request(2)] })).unwrap();
    assert_eq!(requests.listed(first.clone()).len(), 2);
    // Listed again: nothing new.
    assert!(requests.listed(first.clone()).is_empty());
    requests.answer(Answer {
        id: Uuid::from_u128(1),
        approved: false,
        by: "carol".into(),
        until: None,
    });
    assert_eq!(requests.waiting().len(), 1);
    assert!(requests.get(Uuid::from_u128(1)).is_none());
    assert_eq!(requests.answers().len(), 1);
    // remotehub still lists it: the answer stays.
    requests.listed(first);
    assert_eq!(requests.answers().len(), 1);
    // It took the answer.
    let later = parsed(&json!({ "requests": [request(2)] })).unwrap();
    requests.listed(later);
    assert!(requests.answers().is_empty());
    assert_eq!(requests.waiting().len(), 1);
}
