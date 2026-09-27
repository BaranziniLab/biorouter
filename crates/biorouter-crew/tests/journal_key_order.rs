//! The journal's hash chain is verified by re-serializing each parsed record (see
//! `replay_record` in `src/broker.rs`). Records written by every shipped broker keep their
//! objects' keys in insertion order, so verification only works when parsing preserves that
//! order. That is serde_json's `preserve_order` feature, which this crate now declares itself.
//!
//! Before it did, the feature arrived only through workspace feature unification when the broker
//! was built in the same cargo invocation as `biorouter-server`. A broker built alone
//! (`cargo build -p biorouter-crew`) sorted keys on re-serialization and refused every existing
//! journal with `journal_corrupt: checksum mismatch` — measured on a live fixture on 2026-09-24.
//! Run this test on its own (`cargo test -p biorouter-crew --test journal_key_order`) so no other
//! package's features can mask a regression.

use serde_json::Value;

#[test]
fn parsed_objects_reserialize_in_the_order_they_were_written() {
    let written = r#"{"zeta":1,"alpha":{"nested_z":true,"nested_a":false},"mid":[{"y":1,"b":2}]}"#;
    let parsed: Value = serde_json::from_str(written).expect("valid JSON");
    assert_eq!(
        serde_json::to_string(&parsed).expect("serializes"),
        written,
        "serde_json must keep insertion order (feature `preserve_order`), or existing journal \
         checksums no longer verify"
    );
}
