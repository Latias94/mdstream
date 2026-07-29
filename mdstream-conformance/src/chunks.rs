//! Deterministic, UTF-8-safe chunk schedules used by conformance fixtures.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Range;

/// Default cap used when exhaustively enumerating UTF-8 chunk partitions.
pub const DEFAULT_EXHAUSTIVE_PARTITION_LIMIT: usize = 4_096;

/// Absolute allocation cap for exhaustive partition enumeration.
pub const MAX_EXHAUSTIVE_PARTITIONS: usize = 65_536;

/// Stable identifier for the seeded schedule algorithm.
pub const SEEDED_SCHEDULE_ALGORITHM: &str = "fnv1a64-xorshift64-v1";

/// Markdown shapes used by the canonical pending-source scenarios.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingScenarioShape {
    /// A single unfinished paragraph.
    Paragraph,
    /// An unclosed fenced code block.
    Fence,
    /// Repeated block quote container lines.
    Container,
    /// A GFM table header followed by repeated rows.
    Table,
    /// A paragraph containing multibyte Unicode scalars.
    Unicode,
}

impl PendingScenarioShape {
    /// Every canonical pending-source shape in stable selection order.
    pub const ALL: [Self; 5] = [
        Self::Paragraph,
        Self::Fence,
        Self::Container,
        Self::Table,
        Self::Unicode,
    ];

    /// Returns the stable identifier for this Markdown shape.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Fence => "fence",
            Self::Container => "container",
            Self::Table => "table",
            Self::Unicode => "unicode",
        }
    }
}

/// A fixed byte size used by the canonical pending-source scenarios.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingScenarioSize(usize);

impl PendingScenarioSize {
    /// 8 KiB.
    pub const KIB_8: Self = Self(8 * 1024);
    /// 16 KiB.
    pub const KIB_16: Self = Self(16 * 1024);
    /// 32 KiB.
    pub const KIB_32: Self = Self(32 * 1024);
    /// 64 KiB.
    pub const KIB_64: Self = Self(64 * 1024);

    /// Every canonical pending-source size in ascending order.
    pub const ALL: [Self; 4] = [Self::KIB_8, Self::KIB_16, Self::KIB_32, Self::KIB_64];

    /// Returns the target UTF-8 byte length.
    pub const fn bytes(self) -> usize {
        self.0
    }
}

/// One canonical pending Markdown shape at one fixed byte size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalPendingScenario {
    shape: PendingScenarioShape,
    size: PendingScenarioSize,
}

impl CanonicalPendingScenario {
    /// All 20 shape and size combinations in stable size-major order.
    pub const ALL: [Self; 20] = [
        Self::new(PendingScenarioShape::Paragraph, PendingScenarioSize::KIB_8),
        Self::new(PendingScenarioShape::Fence, PendingScenarioSize::KIB_8),
        Self::new(PendingScenarioShape::Container, PendingScenarioSize::KIB_8),
        Self::new(PendingScenarioShape::Table, PendingScenarioSize::KIB_8),
        Self::new(PendingScenarioShape::Unicode, PendingScenarioSize::KIB_8),
        Self::new(PendingScenarioShape::Paragraph, PendingScenarioSize::KIB_16),
        Self::new(PendingScenarioShape::Fence, PendingScenarioSize::KIB_16),
        Self::new(PendingScenarioShape::Container, PendingScenarioSize::KIB_16),
        Self::new(PendingScenarioShape::Table, PendingScenarioSize::KIB_16),
        Self::new(PendingScenarioShape::Unicode, PendingScenarioSize::KIB_16),
        Self::new(PendingScenarioShape::Paragraph, PendingScenarioSize::KIB_32),
        Self::new(PendingScenarioShape::Fence, PendingScenarioSize::KIB_32),
        Self::new(PendingScenarioShape::Container, PendingScenarioSize::KIB_32),
        Self::new(PendingScenarioShape::Table, PendingScenarioSize::KIB_32),
        Self::new(PendingScenarioShape::Unicode, PendingScenarioSize::KIB_32),
        Self::new(PendingScenarioShape::Paragraph, PendingScenarioSize::KIB_64),
        Self::new(PendingScenarioShape::Fence, PendingScenarioSize::KIB_64),
        Self::new(PendingScenarioShape::Container, PendingScenarioSize::KIB_64),
        Self::new(PendingScenarioShape::Table, PendingScenarioSize::KIB_64),
        Self::new(PendingScenarioShape::Unicode, PendingScenarioSize::KIB_64),
    ];

    /// Selects one canonical shape and size combination.
    pub const fn new(shape: PendingScenarioShape, size: PendingScenarioSize) -> Self {
        Self { shape, size }
    }

    /// Returns the stable identifier for this shape and size combination.
    pub const fn id(self) -> &'static str {
        use PendingScenarioShape::{Container, Fence, Paragraph, Table, Unicode};

        match (self.shape, self.size.bytes()) {
            (Paragraph, 8_192) => "paragraph-8kib",
            (Fence, 8_192) => "fence-8kib",
            (Container, 8_192) => "container-8kib",
            (Table, 8_192) => "table-8kib",
            (Unicode, 8_192) => "unicode-8kib",
            (Paragraph, 16_384) => "paragraph-16kib",
            (Fence, 16_384) => "fence-16kib",
            (Container, 16_384) => "container-16kib",
            (Table, 16_384) => "table-16kib",
            (Unicode, 16_384) => "unicode-16kib",
            (Paragraph, 32_768) => "paragraph-32kib",
            (Fence, 32_768) => "fence-32kib",
            (Container, 32_768) => "container-32kib",
            (Table, 32_768) => "table-32kib",
            (Unicode, 32_768) => "unicode-32kib",
            (Paragraph, 65_536) => "paragraph-64kib",
            (Fence, 65_536) => "fence-64kib",
            (Container, 65_536) => "container-64kib",
            (Table, 65_536) => "table-64kib",
            (Unicode, 65_536) => "unicode-64kib",
            _ => unreachable!(),
        }
    }

    /// Returns this scenario's Markdown shape.
    pub const fn shape(self) -> PendingScenarioShape {
        self.shape
    }

    /// Returns this scenario's fixed UTF-8 byte length.
    pub const fn target_bytes(self) -> usize {
        self.size.bytes()
    }

    /// Generates the canonical UTF-8 source at exactly the target byte length.
    pub fn source(self) -> String {
        let target_bytes = self.target_bytes();
        match self.shape {
            PendingScenarioShape::Paragraph => "x".repeat(target_bytes),
            PendingScenarioShape::Fence => {
                exact_ascii_fixture("```text\n", "code line\n", target_bytes)
            }
            PendingScenarioShape::Container => {
                let row = format!("> {}\n", "x".repeat(252));
                exact_ascii_fixture("", &row, target_bytes)
            }
            PendingScenarioShape::Table => {
                let row = format!("{} | {}\n", "x".repeat(124), "y".repeat(124));
                exact_ascii_fixture("a | b\n--|--\n", &row, target_bytes)
            }
            PendingScenarioShape::Unicode => {
                let unit = "界x";
                debug_assert_eq!(target_bytes % unit.len(), 0);
                unit.repeat(target_bytes / unit.len())
            }
        }
    }
}

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;
const TRIAL_MIX: u64 = 0x9e37_79b9_7f4a_7c15;
const SEED_MIX: u64 = 0xa076_1d64_78bd_642f;

fn exact_ascii_fixture(prefix: &str, row: &str, target_bytes: usize) -> String {
    debug_assert!(prefix.is_ascii());
    debug_assert!(row.is_ascii() && !row.is_empty());
    debug_assert!(prefix.len() <= target_bytes);

    let mut source = String::with_capacity(target_bytes);
    source.push_str(prefix);
    while source.len() < target_bytes {
        source.push_str(row);
    }
    source.truncate(target_bytes);
    source
}

/// A portable description of how a source document is split into chunks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChunkSchedule {
    /// Deliver the non-empty source as one chunk.
    Whole,
    /// Split after every line feed, preserving CRLF pairs in one chunk.
    Lines,
    /// Deliver one Unicode scalar value per chunk.
    Characters,
    /// Split at explicit internal byte offsets.
    ByteCuts {
        /// Strictly increasing UTF-8 boundaries, excluding zero and source length.
        cuts: Vec<usize>,
    },
    /// Generate deterministic target widths from a stable seeded algorithm.
    Seeded {
        /// Stable fixture or schedule label mixed through FNV-1a.
        label: String,
        /// Caller-controlled seed mixed independently from the trial number.
        #[serde(default)]
        seed: u64,
        /// Reproducible trial number.
        trial: u64,
        /// Maximum generated target width before advancing to a UTF-8 boundary.
        max_bytes: usize,
    },
}

/// Validation failures produced while materializing a chunk schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkError {
    /// A seeded schedule used an invalid zero target width.
    ZeroMaxBytes,
    /// A caller-provided target width was zero.
    ZeroTargetWidth { index: usize },
    /// The width used after caller-provided targets are exhausted was zero.
    ZeroFallbackWidth,
    /// An explicit cut points at byte zero.
    CutAtStart { index: usize },
    /// An explicit cut repeats the implicit source-length endpoint.
    CutAtEnd { index: usize, source_len: usize },
    /// An explicit cut is beyond the source.
    CutOutOfBounds {
        index: usize,
        cut: usize,
        source_len: usize,
    },
    /// Explicit cuts are duplicated or not ordered.
    CutsNotStrictlyIncreasing {
        index: usize,
        previous: usize,
        current: usize,
    },
    /// An explicit cut would split a UTF-8 code point.
    CutNotUtf8Boundary { index: usize, cut: usize },
    /// Exhaustive enumeration was requested with no result budget.
    ZeroPartitionLimit,
    /// Exhaustive enumeration would exceed the caller's result budget.
    PartitionLimitExceeded {
        internal_boundaries: usize,
        required: usize,
        limit: usize,
    },
    /// The exact number of possible partitions cannot fit in `usize`.
    PartitionCountOverflow {
        internal_boundaries: usize,
        limit: usize,
    },
}

impl ChunkSchedule {
    /// Resolve this schedule to byte ranges that cover `source` exactly once.
    ///
    /// Empty input always produces no ranges. For non-empty input, every range
    /// is non-empty and the ranges cover the source exactly once in byte order.
    pub fn ranges(&self, source: &str) -> Result<Vec<Range<usize>>, ChunkError> {
        match self {
            Self::Whole => Ok(whole_ranges(source)),
            Self::Lines => Ok(line_ranges(source)),
            Self::Characters => Ok(character_ranges(source)),
            Self::ByteCuts { cuts } => explicit_ranges(source, cuts),
            Self::Seeded {
                label,
                seed,
                trial,
                max_bytes,
            } => seeded_ranges(source, label, *seed, *trial, *max_bytes),
        }
    }

    /// Resolve this schedule to borrowed chunks from `source`.
    pub fn slices<'source>(&self, source: &'source str) -> Result<Vec<&'source str>, ChunkError> {
        self.ranges(source)
            .map(|ranges| ranges.into_iter().map(|range| &source[range]).collect())
    }
}

impl fmt::Display for ChunkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMaxBytes => formatter.write_str("seeded max_bytes must be greater than zero"),
            Self::ZeroTargetWidth { index } => {
                write!(
                    formatter,
                    "target width at index {index} must be greater than zero"
                )
            }
            Self::ZeroFallbackWidth => {
                formatter.write_str("fallback target width must be greater than zero")
            }
            Self::CutAtStart { index } => {
                write!(
                    formatter,
                    "cut at index {index} is zero; byte zero is an implicit endpoint"
                )
            }
            Self::CutAtEnd { index, source_len } => write!(
                formatter,
                "cut at index {index} equals source length {source_len}; the final endpoint is implicit"
            ),
            Self::CutOutOfBounds {
                index,
                cut,
                source_len,
            } => write!(
                formatter,
                "cut at index {index} is byte {cut}, beyond source length {source_len}"
            ),
            Self::CutsNotStrictlyIncreasing {
                index,
                previous,
                current,
            } => write!(
                formatter,
                "cut at index {index} is byte {current}, which is not greater than previous byte {previous}"
            ),
            Self::CutNotUtf8Boundary { index, cut } => write!(
                formatter,
                "cut at index {index} is byte {cut}, which is not a UTF-8 boundary"
            ),
            Self::ZeroPartitionLimit => {
                formatter.write_str("exhaustive partition limit must be greater than zero")
            }
            Self::PartitionLimitExceeded {
                internal_boundaries,
                required,
                limit,
            } => write!(
                formatter,
                "{internal_boundaries} internal UTF-8 boundaries require {required} partitions, exceeding limit {limit}"
            ),
            Self::PartitionCountOverflow {
                internal_boundaries,
                limit,
            } => write!(
                formatter,
                "partition count for {internal_boundaries} internal UTF-8 boundaries exceeds usize and limit {limit}"
            ),
        }
    }
}

impl std::error::Error for ChunkError {}

/// Enumerate every legal UTF-8 chunk partition using the default safety cap.
pub fn exhaustive_utf8_partitions(source: &str) -> Result<Vec<ChunkSchedule>, ChunkError> {
    exhaustive_utf8_partitions_with_limit(source, DEFAULT_EXHAUSTIVE_PARTITION_LIMIT)
}

/// Enumerate every legal UTF-8 chunk partition within an explicit result cap.
///
/// The effective cap never exceeds [`MAX_EXHAUSTIVE_PARTITIONS`], even when a
/// caller supplies a larger limit.
pub fn exhaustive_utf8_partitions_with_limit(
    source: &str,
    limit: usize,
) -> Result<Vec<ChunkSchedule>, ChunkError> {
    if limit == 0 {
        return Err(ChunkError::ZeroPartitionLimit);
    }

    let boundaries: Vec<usize> = source
        .char_indices()
        .skip(1)
        .map(|(offset, _)| offset)
        .collect();
    let internal_boundaries = boundaries.len();
    let effective_limit = limit.min(MAX_EXHAUSTIVE_PARTITIONS);
    if internal_boundaries >= usize::BITS as usize {
        return Err(ChunkError::PartitionCountOverflow {
            internal_boundaries,
            limit: effective_limit,
        });
    }

    let required = 1usize << internal_boundaries;
    if required > effective_limit {
        return Err(ChunkError::PartitionLimitExceeded {
            internal_boundaries,
            required,
            limit: effective_limit,
        });
    }

    let mut schedules = Vec::with_capacity(required);
    for mask in 0..required {
        let mut cuts = Vec::with_capacity(mask.count_ones() as usize);
        for (bit, cut) in boundaries.iter().enumerate() {
            if mask & (1usize << bit) != 0 {
                cuts.push(*cut);
            }
        }
        schedules.push(ChunkSchedule::ByteCuts { cuts });
    }
    Ok(schedules)
}

/// Converts caller-provided target widths into an exact UTF-8-safe cover.
///
/// This is intended for fuzzers and captured transport traces whose widths are
/// data rather than a seeded fixture schedule. A target that lands inside a
/// code point advances to the next UTF-8 boundary. Once `widths` is exhausted,
/// `fallback_width` is used for the remaining source.
pub fn utf8_ranges_from_target_widths(
    source: &str,
    widths: impl IntoIterator<Item = usize>,
    fallback_width: usize,
) -> Result<Vec<Range<usize>>, ChunkError> {
    if fallback_width == 0 {
        return Err(ChunkError::ZeroFallbackWidth);
    }

    let mut widths = widths.into_iter().enumerate();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let width = match widths.next() {
            Some((index, 0)) => return Err(ChunkError::ZeroTargetWidth { index }),
            Some((_, width)) => width,
            None => fallback_width,
        };
        let mut end = start.saturating_add(width).min(source.len());
        while end < source.len() && !source.is_char_boundary(end) {
            end += 1;
        }
        ranges.push(start..end);
        start = end;
    }
    Ok(ranges)
}

fn whole_ranges(source: &str) -> Vec<Range<usize>> {
    (!source.is_empty())
        .then_some(0..source.len())
        .into_iter()
        .collect()
}

fn line_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push(start..offset + 1);
            start = offset + 1;
        }
    }
    if start < source.len() {
        ranges.push(start..source.len());
    }
    ranges
}

fn character_ranges(source: &str) -> Vec<Range<usize>> {
    source
        .char_indices()
        .map(|(start, character)| start..start + character.len_utf8())
        .collect()
}

fn explicit_ranges(source: &str, cuts: &[usize]) -> Result<Vec<Range<usize>>, ChunkError> {
    let mut previous = None;
    for (index, cut) in cuts.iter().copied().enumerate() {
        if cut == 0 {
            return Err(ChunkError::CutAtStart { index });
        }
        if cut == source.len() {
            return Err(ChunkError::CutAtEnd {
                index,
                source_len: source.len(),
            });
        }
        if cut > source.len() {
            return Err(ChunkError::CutOutOfBounds {
                index,
                cut,
                source_len: source.len(),
            });
        }
        if let Some(previous) = previous {
            if cut <= previous {
                return Err(ChunkError::CutsNotStrictlyIncreasing {
                    index,
                    previous,
                    current: cut,
                });
            }
        }
        if !source.is_char_boundary(cut) {
            return Err(ChunkError::CutNotUtf8Boundary { index, cut });
        }
        previous = Some(cut);
    }

    if source.is_empty() {
        return Ok(Vec::new());
    }

    let mut ranges = Vec::with_capacity(cuts.len() + 1);
    let mut start = 0;
    for cut in cuts {
        ranges.push(start..*cut);
        start = *cut;
    }
    ranges.push(start..source.len());
    Ok(ranges)
}

fn seeded_ranges(
    source: &str,
    label: &str,
    seed: u64,
    trial: u64,
    max_bytes: usize,
) -> Result<Vec<Range<usize>>, ChunkError> {
    if max_bytes == 0 {
        return Err(ChunkError::ZeroMaxBytes);
    }

    let mut state = fnv1a64(label) ^ seed.wrapping_mul(SEED_MIX) ^ trial.wrapping_mul(TRIAL_MIX);
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let want = (xorshift64(&mut state) % max_bytes as u64) as usize + 1;
        let mut end = start.saturating_add(want).min(source.len());
        while end < source.len() && !source.is_char_boundary(end) {
            end += 1;
        }
        ranges.push(start..end);
        start = end;
    }
    Ok(ranges)
}

fn fnv1a64(label: &str) -> u64 {
    let mut hash = FNV1A_OFFSET_BASIS;
    for byte in label.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

fn xorshift64(state: &mut u64) -> u64 {
    let mut value = *state;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *state = value;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_exact_cover(schedule: &ChunkSchedule, source: &str) {
        let ranges = schedule.ranges(source).expect("schedule should be valid");
        let mut cursor = 0;
        for range in &ranges {
            assert_eq!(range.start, cursor);
            assert!(
                range.start < range.end,
                "non-empty sources use non-empty chunks"
            );
            assert!(source.is_char_boundary(range.start));
            assert!(source.is_char_boundary(range.end));
            cursor = range.end;
        }
        assert_eq!(cursor, source.len());

        let rebuilt = schedule
            .slices(source)
            .expect("schedule should produce slices")
            .concat();
        assert_eq!(rebuilt, source);
    }

    #[test]
    fn standard_schedules_cover_multibyte_and_crlf_source() {
        let source = "a\r\n中🙂";

        assert_eq!(ChunkSchedule::Whole.ranges(source).unwrap(), vec![0..10]);
        assert_eq!(
            ChunkSchedule::Lines.ranges(source).unwrap(),
            vec![0..3, 3..10]
        );
        assert_eq!(
            ChunkSchedule::Characters.ranges(source).unwrap(),
            vec![0..1, 1..2, 2..3, 3..6, 6..10]
        );

        for schedule in [
            ChunkSchedule::Whole,
            ChunkSchedule::Lines,
            ChunkSchedule::Characters,
        ] {
            assert_exact_cover(&schedule, source);
        }
    }

    #[test]
    fn every_schedule_delivers_no_chunk_for_empty_source() {
        let schedules = [
            ChunkSchedule::Whole,
            ChunkSchedule::Lines,
            ChunkSchedule::Characters,
            ChunkSchedule::ByteCuts { cuts: Vec::new() },
            ChunkSchedule::Seeded {
                label: "empty".into(),
                seed: 7,
                trial: 3,
                max_bytes: 4,
            },
        ];

        for schedule in schedules {
            assert!(schedule.ranges("").unwrap().is_empty());
            assert!(schedule.slices("").unwrap().is_empty());
        }
    }

    #[test]
    fn explicit_byte_cuts_are_internal_utf8_boundaries() {
        let source = "a中🙂z";
        let schedule = ChunkSchedule::ByteCuts {
            cuts: vec![1, 4, 8],
        };

        assert_eq!(
            schedule.ranges(source).unwrap(),
            vec![0..1, 1..4, 4..8, 8..9]
        );
        assert_exact_cover(&schedule, source);
    }

    #[test]
    fn explicit_byte_cuts_report_the_failing_cut_and_index() {
        let source = "a中🙂z";

        let cases = [
            (vec![0], ChunkError::CutAtStart { index: 0 }),
            (
                vec![source.len()],
                ChunkError::CutAtEnd {
                    index: 0,
                    source_len: source.len(),
                },
            ),
            (
                vec![source.len() + 1],
                ChunkError::CutOutOfBounds {
                    index: 0,
                    cut: source.len() + 1,
                    source_len: source.len(),
                },
            ),
            (
                vec![4, 1],
                ChunkError::CutsNotStrictlyIncreasing {
                    index: 1,
                    previous: 4,
                    current: 1,
                },
            ),
            (
                vec![1, 1],
                ChunkError::CutsNotStrictlyIncreasing {
                    index: 1,
                    previous: 1,
                    current: 1,
                },
            ),
            (vec![2], ChunkError::CutNotUtf8Boundary { index: 0, cut: 2 }),
        ];

        for (cuts, expected) in cases {
            let error = ChunkSchedule::ByteCuts { cuts }
                .ranges(source)
                .expect_err("invalid cuts must be rejected");
            assert_eq!(error, expected);
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn seeded_schedule_is_reproducible_and_has_a_fixed_vector() {
        let schedule = ChunkSchedule::Seeded {
            label: "fixture".into(),
            seed: 7,
            trial: 3,
            max_bytes: 3,
        };
        let source = "a中🙂bc";

        let first = schedule.ranges(source).unwrap();
        assert_eq!(schedule.ranges(source).unwrap(), first);
        assert_eq!(first, vec![0..1, 1..4, 4..8, 8..10]);
        assert_exact_cover(&schedule, source);

        let changed_trial = ChunkSchedule::Seeded {
            label: "fixture".into(),
            seed: 7,
            trial: 4,
            max_bytes: 3,
        };
        assert_ne!(changed_trial.ranges(source).unwrap(), first);
    }

    #[test]
    fn seeded_schedule_rejects_zero_max_bytes_even_for_empty_source() {
        let schedule = ChunkSchedule::Seeded {
            label: "invalid".into(),
            seed: 0,
            trial: 0,
            max_bytes: 0,
        };

        assert_eq!(schedule.ranges("").unwrap_err(), ChunkError::ZeroMaxBytes);
    }

    #[test]
    fn exhaustive_partitions_include_every_internal_utf8_cut_combination() {
        let schedules = exhaustive_utf8_partitions_with_limit("aé🙂", 4).unwrap();
        let cuts: Vec<Vec<usize>> = schedules
            .iter()
            .map(|schedule| match schedule {
                ChunkSchedule::ByteCuts { cuts } => cuts.clone(),
                other => panic!("unexpected exhaustive schedule: {other:?}"),
            })
            .collect();

        assert_eq!(cuts, vec![vec![], vec![1], vec![3], vec![1, 3]]);
        for schedule in schedules {
            assert_exact_cover(&schedule, "aé🙂");
        }
    }

    #[test]
    fn exhaustive_empty_source_has_one_empty_partition() {
        let schedules = exhaustive_utf8_partitions("").unwrap();
        assert_eq!(
            schedules,
            vec![ChunkSchedule::ByteCuts { cuts: Vec::new() }]
        );
        assert!(schedules[0].ranges("").unwrap().is_empty());
    }

    #[test]
    fn exhaustive_partitions_fail_before_combinatorial_explosion() {
        assert_eq!(
            exhaustive_utf8_partitions_with_limit("abcdef", 31).unwrap_err(),
            ChunkError::PartitionLimitExceeded {
                internal_boundaries: 5,
                required: 32,
                limit: 31,
            }
        );
        assert_eq!(
            exhaustive_utf8_partitions_with_limit("a", 0).unwrap_err(),
            ChunkError::ZeroPartitionLimit
        );

        let hard_limited_source = "a".repeat(18);
        assert_eq!(
            exhaustive_utf8_partitions_with_limit(&hard_limited_source, usize::MAX).unwrap_err(),
            ChunkError::PartitionLimitExceeded {
                internal_boundaries: 17,
                required: 131_072,
                limit: MAX_EXHAUSTIVE_PARTITIONS,
            }
        );

        let overflowing_source = "a".repeat(usize::BITS as usize + 1);
        assert_eq!(
            exhaustive_utf8_partitions_with_limit(&overflowing_source, usize::MAX).unwrap_err(),
            ChunkError::PartitionCountOverflow {
                internal_boundaries: usize::BITS as usize,
                limit: MAX_EXHAUSTIVE_PARTITIONS,
            }
        );
    }

    #[test]
    fn schedules_have_a_stable_tagged_json_shape() {
        let schedule = ChunkSchedule::Seeded {
            label: "case".into(),
            seed: 0,
            trial: 2,
            max_bytes: 16,
        };
        let value = serde_json::to_value(&schedule).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "seeded",
                "label": "case",
                "seed": 0,
                "trial": 2,
                "max_bytes": 16
            })
        );
        assert_eq!(
            serde_json::from_value::<ChunkSchedule>(value).unwrap(),
            schedule
        );

        let without_seed = serde_json::json!({
            "kind": "seeded",
            "label": "case",
            "trial": 2,
            "max_bytes": 16
        });
        assert_eq!(
            serde_json::from_value::<ChunkSchedule>(without_seed).unwrap(),
            schedule
        );

        let unknown_field = serde_json::json!({
            "kind": "byte_cuts",
            "cuts": [],
            "unexpected": true
        });
        assert!(serde_json::from_value::<ChunkSchedule>(unknown_field).is_err());
    }

    #[test]
    fn target_widths_share_utf8_boundary_handling_with_fixture_schedules() {
        let source = "a中🙂bc";
        let ranges = utf8_ranges_from_target_widths(source, [1, 1, 2], 3).unwrap();
        assert_eq!(ranges, vec![0..1, 1..4, 4..8, 8..10]);
        assert_eq!(
            utf8_ranges_from_target_widths(source, [1, 0], 3).unwrap_err(),
            ChunkError::ZeroTargetWidth { index: 1 }
        );
        assert_eq!(
            utf8_ranges_from_target_widths(source, [1], 0).unwrap_err(),
            ChunkError::ZeroFallbackWidth
        );
    }

    #[test]
    fn canonical_pending_scenarios_have_stable_ids_and_exact_utf8_lengths() {
        let expected_ids = [
            "paragraph-8kib",
            "fence-8kib",
            "container-8kib",
            "table-8kib",
            "unicode-8kib",
            "paragraph-16kib",
            "fence-16kib",
            "container-16kib",
            "table-16kib",
            "unicode-16kib",
            "paragraph-32kib",
            "fence-32kib",
            "container-32kib",
            "table-32kib",
            "unicode-32kib",
            "paragraph-64kib",
            "fence-64kib",
            "container-64kib",
            "table-64kib",
            "unicode-64kib",
        ];

        assert_eq!(CanonicalPendingScenario::ALL.len(), expected_ids.len());
        for (scenario, expected_id) in CanonicalPendingScenario::ALL.into_iter().zip(expected_ids) {
            assert_eq!(scenario.id(), expected_id);
            let source = scenario.source();
            assert_eq!(source.len(), scenario.target_bytes(), "{expected_id}");

            match scenario.shape() {
                PendingScenarioShape::Paragraph => assert!(!source.contains('\n')),
                PendingScenarioShape::Fence => assert!(source.starts_with("```text\n")),
                PendingScenarioShape::Container => assert!(source.starts_with("> ")),
                PendingScenarioShape::Table => assert!(source.starts_with("a | b\n--|--\n")),
                PendingScenarioShape::Unicode => assert!(source.starts_with("界x")),
            }
        }
    }
}
