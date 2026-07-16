//! Shared `[version_byte || bincode(body)]` framing for the in-vault payloads
//! that carry it: the HD-seed [`StoredSeedEnvelope`] and the imported
//! [`SingleKeyEntry`]. Encoding prepends a leading version tag; decoding reads
//! it back and, when the tag or body does not match, defers to a per-format
//! fallback closure (a bare-bincode legacy value, a raw fixed-length blob, or a
//! typed error) so each payload keeps its own legacy-shape handling.
//!
//! [`StoredSeedEnvelope`]: crate::model::wallet::seed_envelope::StoredSeedEnvelope
//! [`SingleKeyEntry`]: crate::wallet_backend::single_key_entry::SingleKeyEntry

use serde::Serialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

/// Encode `body` as `[version || bincode(body)]` in a zeroizing allocation.
pub(crate) fn encode_tagged<T: Serialize>(
    version: u8,
    body: &T,
) -> Result<Zeroizing<Vec<u8>>, bincode::error::EncodeError> {
    let config = bincode::config::standard();
    let mut size_writer = bincode::enc::write::SizeWriter::default();
    bincode::serde::encode_into_writer(body, &mut size_writer, config)?;

    let mut out = Zeroizing::new(vec![0; size_writer.bytes_written + 1]);
    out[0] = version;
    let written = bincode::serde::encode_into_slice(body, &mut out[1..], config)?;
    out.truncate(written + 1);
    Ok(out)
}

/// Decode a `[version || bincode(T)]` payload. When `bytes` begins with
/// `version` and the remainder decodes cleanly as `T`, that value is returned;
/// otherwise the call defers to `fallback` with the full `bytes` (the legacy /
/// untagged path — which may itself decode a bare-bincode value, interpret a
/// raw blob, or return a typed error).
pub(crate) fn decode_tagged_or<T, E>(
    bytes: &[u8],
    version: u8,
    fallback: impl FnOnce(&[u8]) -> Result<T, E>,
) -> Result<T, E>
where
    T: DeserializeOwned,
{
    if let Some((&tag, rest)) = bytes.split_first()
        && tag == version
        && let Ok((decoded, _)) =
            bincode::serde::decode_from_slice::<T, _>(rest, bincode::config::standard())
    {
        return Ok(decoded);
    }
    fallback(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        a: u32,
        b: String,
    }

    const VERSION: u8 = 7;

    #[test]
    fn tagged_round_trip() {
        let s = Sample {
            a: 42,
            b: "hi".into(),
        };
        let bytes = encode_tagged(VERSION, &s).expect("encode");
        assert_eq!(bytes[0], VERSION, "payload starts with the version tag");
        let got: Sample =
            decode_tagged_or(&bytes, VERSION, |_| Err::<Sample, ()>(())).expect("decode");
        assert_eq!(got, s);
    }

    #[test]
    fn tagged_encoding_is_zeroizing_and_wire_compatible() {
        let sample = Sample {
            a: 42,
            b: "hi".into(),
        };
        let body = bincode::serde::encode_to_vec(&sample, bincode::config::standard())
            .expect("reference encode");
        let mut expected = vec![VERSION];
        expected.extend_from_slice(&body);

        let actual: zeroize::Zeroizing<Vec<u8>> = encode_tagged(VERSION, &sample).expect("encode");

        assert_eq!(&*actual, &expected);
    }

    #[test]
    fn wrong_tag_defers_to_fallback() {
        let s = Sample {
            a: 1,
            b: "x".into(),
        };
        let bytes = encode_tagged(VERSION, &s).expect("encode");
        // Decode with a different expected version — the tag mismatches, so the
        // fallback runs and receives the full byte slice.
        let got: Result<Sample, u8> = decode_tagged_or(&bytes, VERSION + 1, |b| Err(b[0]));
        assert_eq!(got, Err(VERSION), "fallback saw the untouched leading byte");
    }

    #[test]
    fn empty_input_defers_to_fallback() {
        let got: Result<Sample, &str> = decode_tagged_or(&[], VERSION, |_| Err("empty"));
        assert_eq!(got, Err("empty"));
    }
}
