mod builder;
mod effects;
mod lifecycle;

pub use builder::StreamEngineBuilder;
pub use effects::EngineOutput;
pub use lifecycle::EngineError;

use mdstream_protocol::{
    ApplyOutcome, Coordinate, DocumentLifecycle, Epoch, ProtocolError, ProtocolLimits, Reducer,
    Snapshot,
};

use self::effects::FrameShell;
use self::lifecycle::{append_change, reset_change, source_end};
use crate::boundary::BoundaryPlugin;
use crate::options::Options;
use crate::stream::{LegacyFramer, NewlineNormalizer};
use crate::transform::PendingTransformer;
use crate::types::{Block, Update};

#[derive(Debug)]
pub struct StreamEngine {
    framer: LegacyFramer,
    normalizer: NewlineNormalizer,
    producer: Reducer,
    initial_epoch: Epoch,
    frame: FrameShell,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Retained scanner memory and absolute compaction progress.
pub struct EngineMetrics {
    pub retained_input_bytes: usize,
    pub retained_source_base: u64,
}

impl StreamEngine {
    pub fn new(options: Options) -> Self {
        Self::with_limits(options, ProtocolLimits::default())
    }

    fn with_limits(options: Options, limits: ProtocolLimits) -> Self {
        Self {
            framer: LegacyFramer::new(options),
            normalizer: NewlineNormalizer::default(),
            producer: Reducer::with_limits(limits),
            initial_epoch: Epoch::new(1),
            frame: FrameShell::default(),
        }
    }

    pub(crate) fn new_legacy(options: Options) -> Self {
        let limits = ProtocolLimits {
            max_source_bytes: usize::MAX,
            ..ProtocolLimits::default()
        };
        Self::with_limits(options, limits)
    }

    pub fn builder(options: Options) -> StreamEngineBuilder {
        StreamEngineBuilder::new(options)
    }

    pub fn streamdown_defaults() -> Self {
        StreamEngineBuilder::streamdown_defaults().build()
    }

    pub fn append(&mut self, chunk: &str) -> Result<EngineOutput, EngineError> {
        self.append_transition(chunk).map(|(output, _)| output)
    }

    pub fn finish(&mut self) -> Result<EngineOutput, EngineError> {
        self.finish_transition().map(|(output, _)| output)
    }

    pub fn reset(&mut self) -> Result<EngineOutput, EngineError> {
        let change = reset_change(&self.producer, self.initial_epoch)?;
        apply_canonical(&mut self.producer, &change)?;

        self.framer.reset();
        self.normalizer = NewlineNormalizer::default();
        self.frame = FrameShell::default();
        Ok(EngineOutput::one(change))
    }

    pub fn lifecycle(&self) -> DocumentLifecycle {
        self.producer
            .document()
            .map_or(DocumentLifecycle::Open, |document| document.lifecycle())
    }

    pub fn coordinate(&self) -> Option<&Coordinate> {
        self.producer
            .document()
            .map(mdstream_protocol::Document::coordinate)
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.producer.document().map(|document| document.snapshot())
    }

    pub fn metrics(&self) -> EngineMetrics {
        EngineMetrics {
            retained_input_bytes: self.framer.buffer().len(),
            retained_source_base: u64::try_from(self.framer.retained_source_base())
                .expect("retained source offsets fit the protocol cursor domain"),
        }
    }

    fn append_transition(&mut self, chunk: &str) -> Result<(EngineOutput, Update), EngineError> {
        if self.lifecycle() == DocumentLifecycle::Finalized {
            return Err(EngineError::Finished);
        }

        let (normalizer, suffix) = self.normalizer.append(chunk);
        if suffix.is_empty() {
            self.normalizer = normalizer;
            return Ok((EngineOutput::default(), Update::empty()));
        }

        let source_end = source_end(&self.producer, &suffix)?;
        let (operations, frame) = self.frame.append(source_end);
        let change = append_change(&self.producer, self.initial_epoch, suffix, operations)?;
        apply_canonical(&mut self.producer, &change)?;
        let legacy = self.framer.append_normalized(&change.source().suffix);

        self.normalizer = normalizer;
        self.frame = frame;
        Ok((EngineOutput::one(change), legacy))
    }

    fn finish_transition(&mut self) -> Result<(EngineOutput, Update), EngineError> {
        if self.lifecycle() == DocumentLifecycle::Finalized {
            return Ok((EngineOutput::default(), Update::empty()));
        }

        let (normalizer, suffix) = self.normalizer.finish();
        let source_end = source_end(&self.producer, &suffix)?;
        let (operations, frame) = self.frame.finish(source_end);
        let change = append_change(&self.producer, self.initial_epoch, suffix, operations)?;
        apply_canonical(&mut self.producer, &change)?;
        let legacy = self.framer.finish(&change.source().suffix);

        self.normalizer = normalizer;
        self.frame = frame;
        Ok((EngineOutput::one(change), legacy))
    }

    pub(crate) fn append_legacy(&mut self, chunk: &str) -> Update {
        match self.append_transition(chunk) {
            Ok((_, update)) => update,
            Err(EngineError::Finished) => Update::empty(),
            Err(error) => panic!("legacy stream bridge failed: {error}"),
        }
    }

    pub(crate) fn finish_legacy(&mut self) -> Update {
        self.finish_transition()
            .unwrap_or_else(|error| panic!("legacy stream bridge failed: {error}"))
            .1
    }

    pub(crate) fn reset_legacy(&mut self) {
        self.reset()
            .unwrap_or_else(|error| panic!("legacy stream bridge failed: {error}"));
    }

    pub(crate) fn push_pending_transformer_legacy<T>(&mut self, transformer: T)
    where
        T: PendingTransformer + 'static,
    {
        self.framer.push_pending_transformer(transformer);
    }

    pub(crate) fn push_boundary_plugin_legacy<T>(&mut self, plugin: T)
    where
        T: BoundaryPlugin + 'static,
    {
        self.framer.push_boundary_plugin(plugin);
    }

    pub(crate) fn legacy_buffer(&self) -> &str {
        self.framer.buffer()
    }

    pub(crate) fn legacy_snapshot_blocks(&mut self) -> Vec<Block> {
        self.framer.snapshot_blocks()
    }
}

impl Default for StreamEngine {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

fn apply_canonical(
    reducer: &mut Reducer,
    change: &mdstream_protocol::ChangeSet,
) -> Result<(), EngineError> {
    let outcome = reducer
        .apply_producer(change.clone())
        .map_err(classify_producer_error)?;
    if !matches!(
        outcome,
        ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
    ) {
        return Err(EngineError::InternalInvariant(
            ProtocolError::InvalidChange(format!("producer reducer returned {outcome:?}")),
        ));
    }
    Ok(())
}

fn classify_producer_error(error: ProtocolError) -> EngineError {
    match error {
        error @ (ProtocolError::CursorOverflow
        | ProtocolError::MetadataOverflow
        | ProtocolError::SourceTooLarge { .. }
        | ProtocolError::TooManyNodes { .. }
        | ProtocolError::TooManyOperations { .. }
        | ProtocolError::ValueTooLarge { .. }) => EngineError::Protocol(error),
        error => EngineError::InternalInvariant(error),
    }
}
