use std::{
    fmt,
    io::{self, Write},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};

macro_rules! decimal_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            pub fn checked_add(self, value: u64) -> Option<Self> {
                self.0.checked_add(value).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = DecimalIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_decimal(value).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(de::Error::custom)
            }
        }
    };
}

decimal_id!(Epoch);
decimal_id!(Sequence);
decimal_id!(SourceCursor);
decimal_id!(NodeId);
decimal_id!(ResourceId);
decimal_id!(RequestGeneration);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalIdError {
    Empty,
    InvalidSyntax,
    LeadingZero,
    Overflow,
}

impl fmt::Display for DecimalIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "decimal identifier cannot be empty",
            Self::InvalidSyntax => "decimal identifier must contain ASCII digits only",
            Self::LeadingZero => "decimal identifier cannot contain a leading zero",
            Self::Overflow => "decimal identifier exceeds u64",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for DecimalIdError {}

fn parse_decimal(value: &str) -> Result<u64, DecimalIdError> {
    if value.is_empty() {
        return Err(DecimalIdError::Empty);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DecimalIdError::InvalidSyntax);
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(DecimalIdError::LeadingZero);
    }
    value.parse().map_err(|_| DecimalIdError::Overflow)
}

macro_rules! opaque_id {
    ($name:ident, $domain:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, OpaqueIdError> {
                let value = value.into();
                validate_opaque(&value)?;
                Ok(Self(value))
            }

            pub fn digest(bytes: &[u8]) -> Self {
                let mut digest = Sha256::new();
                digest.update($domain.as_bytes());
                digest.update([0]);
                digest.update(bytes);
                Self(format_digest(digest))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

opaque_id!(ChangeId, "mdstream.change-id/v1");
opaque_id!(NodeVersion, "mdstream.node-version/v1");
opaque_id!(ProcessorInputVersion, "mdstream.processor-input-version/v1");
opaque_id!(ResourceVersion, "mdstream.resource-version/v1");
opaque_id!(StructureVersion, "mdstream.structure-version/v1");
opaque_id!(PayloadDigest, "mdstream.change-payload/v1");
opaque_id!(SnapshotDigest, "mdstream.snapshot/v1");

macro_rules! json_digest {
    ($name:ident, $domain:literal) => {
        impl $name {
            pub(crate) fn digest_json<T: Serialize>(value: &T) -> Self {
                let mut digest = Sha256::new();
                digest.update($domain.as_bytes());
                digest.update([0]);
                serde_json::to_writer(DigestWriter(&mut digest), value)
                    .expect("canonical protocol values always serialize");
                Self(format_digest(digest))
            }
        }
    };
}

json_digest!(NodeVersion, "mdstream.node-version/v1");
json_digest!(ProcessorInputVersion, "mdstream.processor-input-version/v1");
json_digest!(ResourceVersion, "mdstream.resource-version/v1");
json_digest!(PayloadDigest, "mdstream.change-payload/v1");
json_digest!(SnapshotDigest, "mdstream.snapshot/v1");

struct DigestWriter<'a>(&'a mut Sha256);

impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn format_digest(digest: Sha256) -> String {
    let bytes = digest.finalize();
    let mut value = String::with_capacity(7 + bytes.len() * 2);
    value.push_str("sha256:");
    for byte in bytes {
        use fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpaqueIdError {
    Empty,
    TooLong,
    InvalidCharacter,
}

impl fmt::Display for OpaqueIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "opaque identifier cannot be empty",
            Self::TooLong => "opaque identifier exceeds 128 bytes",
            Self::InvalidCharacter => "opaque identifier contains an unsupported character",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for OpaqueIdError {}

fn validate_opaque(value: &str) -> Result<(), OpaqueIdError> {
    if value.is_empty() {
        return Err(OpaqueIdError::Empty);
    }
    if value.len() > 128 {
        return Err(OpaqueIdError::TooLong);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(OpaqueIdError::InvalidCharacter);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coordinate {
    pub epoch: Epoch,
    pub sequence: Sequence,
    pub change_id: ChangeId,
    pub source_cursor: SourceCursor,
}
