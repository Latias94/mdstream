use mdstream_processors::{ProcessorLimits, ProcessorLimitsError};

#[test]
fn processor_concurrency_and_slot_limits_must_admit_work() {
    let no_jobs = ProcessorLimits {
        max_in_flight_jobs: 0,
        ..ProcessorLimits::default()
    };
    assert_eq!(
        no_jobs.validate(),
        Err(ProcessorLimitsError::InFlightJobsTooSmall)
    );

    let no_slots = ProcessorLimits {
        max_slots: 0,
        ..ProcessorLimits::default()
    };
    assert_eq!(
        no_slots.validate(),
        Err(ProcessorLimitsError::SlotsTooSmall)
    );
}
