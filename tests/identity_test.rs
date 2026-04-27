use sangha::identity::Identity;

#[tokio::test]
async fn test_bind_once() {
    let id = Identity::new();
    id.bind("sess-1".to_string(), "proj-a".to_string()).expect("first bind ok");
    assert!(id.is_bound());
    assert_eq!(id.session_id().unwrap(), "sess-1");
    assert_eq!(id.project().unwrap(), "proj-a");
}

#[tokio::test]
async fn test_bind_twice_same_id() {
    let id = Identity::new();
    id.bind("sess-1".to_string(), "proj-a".to_string()).expect("first bind ok");
    // Same values — idempotent, should succeed
    id.bind("sess-1".to_string(), "proj-a".to_string()).expect("second bind ok");
    assert_eq!(id.session_id().unwrap(), "sess-1");
}

#[tokio::test]
async fn test_bind_twice_different_id() {
    let id = Identity::new();
    id.bind("sess-1".to_string(), "proj-a".to_string()).expect("first bind ok");
    let err = id.bind("sess-2".to_string(), "proj-a".to_string());
    assert!(err.is_err(), "should reject different session_id");
    // Original binding still intact
    assert_eq!(id.session_id().unwrap(), "sess-1");
}

#[tokio::test]
async fn test_bind_twice_different_project() {
    let id = Identity::new();
    id.bind("sess-1".to_string(), "proj-a".to_string()).expect("first bind ok");
    let err = id.bind("sess-1".to_string(), "proj-b".to_string());
    assert!(err.is_err(), "should reject different project on same session_id");
    assert_eq!(id.project().unwrap(), "proj-a");
}

#[tokio::test]
async fn test_get_before_bind() {
    let id = Identity::new();
    assert!(!id.is_bound());
    let err = id.session_id();
    assert!(err.is_err(), "should error before bind");
    let err = id.project();
    assert!(err.is_err(), "should error before bind");
}

#[tokio::test]
async fn test_get_after_bind() {
    let id = Identity::new();
    id.bind("sess-abc".to_string(), "my-project".to_string()).expect("bind ok");
    assert_eq!(id.session_id().unwrap(), "sess-abc");
    assert_eq!(id.project().unwrap(), "my-project");
}
