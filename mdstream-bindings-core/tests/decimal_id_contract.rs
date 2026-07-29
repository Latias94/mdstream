use mdstream_bindings_core::{
    BINDING_SCHEMA, BindingStatus, ReducerSession, error_payload_json_bytes,
};

#[test]
fn command_decimal_ids_use_the_canonical_invalid_argument_error() {
    let mut reducer = ReducerSession::new(b"").unwrap();
    for request_id in ["", "-1", "1.0", "18446744073709551616"] {
        let command = serde_json::json!({
            "schema": BINDING_SCHEMA,
            "kind": "cancel_processor",
            "request_id": request_id,
        });
        let error = reducer
            .execute(&serde_json::to_vec(&command).unwrap())
            .unwrap_err();
        assert_eq!(error.status(), BindingStatus::InvalidArgument);
        assert_eq!(error.detail_code(), "bindings.decimal_id");
        let envelope: serde_json::Value =
            serde_json::from_slice(&error_payload_json_bytes(&error)).unwrap();
        assert_eq!(envelope["status_name"], "MDSTREAM_INVALID_ARGUMENT");
    }
}

#[test]
fn command_content_ids_use_the_full_u128_range() {
    let mut reducer = ReducerSession::new(b"").unwrap();
    let maximum = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "node_view",
        "node_id": u128::MAX.to_string(),
    });
    let error = reducer
        .execute(&serde_json::to_vec(&maximum).unwrap())
        .unwrap_err();
    assert_ne!(error.detail_code(), "bindings.decimal_id");

    let overflow = serde_json::json!({
        "schema": BINDING_SCHEMA,
        "kind": "node_view",
        "node_id": "340282366920938463463374607431768211456",
    });
    let error = reducer
        .execute(&serde_json::to_vec(&overflow).unwrap())
        .unwrap_err();
    assert_eq!(error.status(), BindingStatus::InvalidArgument);
    assert_eq!(error.detail_code(), "bindings.decimal_id");
}
