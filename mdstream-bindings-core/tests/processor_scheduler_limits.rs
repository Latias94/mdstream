use mdstream_bindings_core::{BINDING_OPTIONS_SCHEMA, ProcessorSchedulerLimits, ReducerSession};

#[test]
fn reducer_reports_effective_default_and_custom_processor_scheduler_limits() {
    let defaults = ReducerSession::new(b"").unwrap();
    assert_eq!(
        defaults.processor_scheduler_limits(),
        ProcessorSchedulerLimits {
            max_in_flight_jobs: 32,
            max_queued_candidates: 256,
        }
    );

    let options = format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "processor":{{
            "max_in_flight_jobs":"2",
            "max_slots":"25"
          }}
        }}"#
    );
    let custom = ReducerSession::new(options.as_bytes()).unwrap();
    assert_eq!(
        custom.processor_scheduler_limits(),
        ProcessorSchedulerLimits {
            max_in_flight_jobs: 2,
            max_queued_candidates: 25,
        }
    );

    let disabled = ReducerSession::new(
        format!(
            r#"{{
              "schema":"{BINDING_OPTIONS_SCHEMA}",
              "processor":{{
                "max_in_flight_jobs":"0",
                "max_slots":"0"
              }}
            }}"#
        )
        .as_bytes(),
    )
    .unwrap_err();
    assert_eq!(
        disabled.message(),
        "processor.max_in_flight_jobs must be at least 1"
    );
}
