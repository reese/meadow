//! Byte-preserving text used for everything derived from Ruby source.
//!
//! Ruby source files are not required to be UTF-8: they may declare another
//! encoding in a magic comment, or contain arbitrary bytes. Converting source
//! slices to `String` would either corrupt those bytes or force a lossy
//! replacement. [`SourceText`] keeps the original bytes and only offers lossy
//! conversion at display boundaries.

use std::borrow::{Borrow, Cow};
use std::fmt;

use serde::{Deserialize, Serialize};

/// An owned, byte-preserving slice of Ruby source text.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceText(Vec<u8>);

impl Serialize for SourceText {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        source_text_serde::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for SourceText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        source_text_serde::deserialize(deserializer)
    }
}

impl SourceText {
    /// Creates a value from a byte slice.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }

    /// Creates a value from owned bytes.
    #[must_use]
    pub const fn from_vec(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the text as UTF-8 when the bytes are valid UTF-8.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Returns a lossy UTF-8 view for display.
    #[must_use]
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    /// Returns true when the bytes contain `needle` as a byte substring.
    #[must_use]
    pub fn contains(&self, needle: &str) -> bool {
        let needle = needle.as_bytes();
        !needle.is_empty() && self.0.windows(needle.len()).any(|window| window == needle)
    }

    /// Returns true when the value has no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl AsRef<[u8]> for SourceText {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Borrow<[u8]> for SourceText {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq<str> for SourceText {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for SourceText {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl From<&str> for SourceText {
    fn from(value: &str) -> Self {
        Self::from_bytes(value.as_bytes())
    }
}

impl From<Vec<u8>> for SourceText {
    fn from(value: Vec<u8>) -> Self {
        Self::from_vec(value)
    }
}

impl fmt::Debug for SourceText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Some(text) => formatter.debug_tuple("SourceText").field(&text).finish(),
            None => formatter.debug_tuple("SourceText").field(&self.0).finish(),
        }
    }
}

impl fmt::Display for SourceText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_string_lossy())
    }
}

/// A byte range in a source file together with its 1-based line range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Byte offset of the first byte in the range.
    pub start: u32,
    /// Byte offset one past the last byte in the range.
    pub end: u32,
    /// 1-based line number where the range starts.
    pub start_line: u32,
    /// 1-based line number where the range ends.
    pub end_line: u32,
}

mod source_text_serde {
    use serde::de::{self, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serializer};

    use super::SourceText;

    pub(super) fn serialize<S>(text: &SourceText, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !serializer.is_human_readable() {
            return serializer.serialize_bytes(text.as_bytes());
        }

        if let Some(value) = text.as_str() {
            serializer.serialize_str(value)
        } else {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("encoding", "bytes")?;
            map.serialize_entry("data", text.as_bytes())?;
            map.end()
        }
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<SourceText, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            deserializer.deserialize_any(SourceTextVisitor)
        } else {
            deserializer.deserialize_bytes(SourceTextBytesVisitor)
        }
    }

    struct SourceTextBytesVisitor;

    impl Visitor<'_> for SourceTextBytesVisitor {
        type Value = SourceText;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a byte string")
        }

        fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(SourceText::from_bytes(value))
        }

        fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(SourceText::from_vec(value))
        }
    }

    struct SourceTextVisitor;

    impl<'de> Visitor<'de> for SourceTextVisitor {
        type Value = SourceText;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a UTF-8 string or a byte payload")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(SourceText::from_bytes(value.as_bytes()))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(SourceText::from_vec(value.into_bytes()))
        }

        fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(SourceText::from_bytes(value))
        }

        fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(SourceText::from_vec(value))
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut encoding: Option<String> = None;
            let mut data: Option<Vec<u8>> = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "encoding" => encoding = Some(map.next_value()?),
                    "data" => data = Some(map.next_value()?),
                    _ => {
                        let _: de::IgnoredAny = map.next_value()?;
                    }
                }
            }

            if encoding.as_deref() == Some("bytes") {
                data.map(SourceText::from_vec)
                    .ok_or_else(|| de::Error::missing_field("data"))
            } else {
                Err(de::Error::custom("invalid SourceText byte payload"))
            }
        }
    }
}
