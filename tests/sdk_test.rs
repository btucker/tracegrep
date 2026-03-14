use tracegrep::sdk::{Error, NodeId, Caller, Reference};

#[test]
fn error_is_send_sync_and_displays() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Error>();

    let err = Error::RepoNotFound {
        path: "/nonexistent".into(),
    };
    let msg = err.to_string();
    assert!(msg.contains("/nonexistent"), "msg: {msg}");
}

#[test]
fn node_id_is_copy_and_debug() {
    fn assert_copy_debug<T: Copy + std::fmt::Debug>() {}
    assert_copy_debug::<NodeId>();
}

#[test]
fn caller_and_reference_have_expected_fields() {
    let caller = Caller {
        file: "src/main.rs".into(),
        function: "main".into(),
        qualified_name: "main".into(),
        line: 1,
        is_test: false,
        depth: 1,
        conditions: vec!["method == POST".into()],
    };
    assert_eq!(caller.file, "src/main.rs");
    assert_eq!(caller.depth, 1);

    let reference = Reference {
        file: "src/main.rs".into(),
        function: "register".into(),
        qualified_name: "register".into(),
        line: 5,
        is_test: false,
        context: Some("passed to register_handler".into()),
    };
    assert!(reference.context.is_some());
}
