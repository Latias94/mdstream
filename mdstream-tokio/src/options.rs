use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct CoalesceOptions {
    /// Flush once a newline is observed in the buffered text.
    pub flush_on_newline: bool,
    /// Flush if no flush happened for this duration (progress guarantee).
    pub max_delay: Duration,
    /// Flush when buffered bytes reach this limit.
    pub max_bytes: usize,
}

impl Default for CoalesceOptions {
    fn default() -> Self {
        Self {
            flush_on_newline: true,
            max_delay: Duration::from_millis(60),
            max_bytes: 8 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CoalescePreset {
    Balanced,
    Fast,
    TimeOnly,
}

impl CoalescePreset {
    pub fn next(self) -> Self {
        match self {
            CoalescePreset::Balanced => CoalescePreset::Fast,
            CoalescePreset::Fast => CoalescePreset::TimeOnly,
            CoalescePreset::TimeOnly => CoalescePreset::Balanced,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CoalescePreset::Balanced => "balanced",
            CoalescePreset::Fast => "fast",
            CoalescePreset::TimeOnly => "time-only",
        }
    }

    pub fn options(self) -> CoalesceOptions {
        match self {
            CoalescePreset::Balanced => CoalesceOptions {
                flush_on_newline: true,
                max_delay: Duration::from_millis(80),
                max_bytes: 16 * 1024,
            },
            CoalescePreset::Fast => CoalesceOptions {
                flush_on_newline: true,
                max_delay: Duration::from_millis(30),
                max_bytes: 4 * 1024,
            },
            CoalescePreset::TimeOnly => CoalesceOptions {
                flush_on_newline: false,
                max_delay: Duration::from_millis(60),
                max_bytes: 4 * 1024,
            },
        }
    }
}
