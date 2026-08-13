//! Fixed-size padding buckets for serialized envelopes, applied before outer
//! AEAD encryption (in `pm-crypto`) so that ciphertext length doesn't leak
//! message size to anything that sees the encrypted blob.

use crate::error::{ProtoError, Result};

/// Bucket sizes in bytes, smallest first. Chosen to match the architecture
/// plan: short text/ACKs fit the 256 B bucket, most text messages fit 1 KB,
/// and 4 KB covers everything else up to the cap.
pub const BUCKETS: [usize; 3] = [256, 1024, 4096];

const LEN_PREFIX_SIZE: usize = 4;

/// Pads `data` into the smallest bucket that fits a 4-byte big-endian length
/// prefix plus `data` itself. Returns an error if `data` doesn't fit even the
/// largest bucket.
pub fn pad(data: &[u8]) -> Result<Vec<u8>> {
    let needed = data.len() + LEN_PREFIX_SIZE;
    let bucket =
        BUCKETS
            .iter()
            .copied()
            .find(|&b| b >= needed)
            .ok_or(ProtoError::TooLargeToPad {
                len: data.len(),
                max: *BUCKETS.last().unwrap() - LEN_PREFIX_SIZE,
            })?;

    let mut out = Vec::with_capacity(bucket);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    out.resize(bucket, 0);
    Ok(out)
}

/// Reverses [`pad`], recovering the original data from a padded buffer.
pub fn unpad(padded: &[u8]) -> Result<Vec<u8>> {
    if padded.len() < LEN_PREFIX_SIZE {
        return Err(ProtoError::PaddedBufferTooShort);
    }
    let declared = u32::from_be_bytes(padded[0..LEN_PREFIX_SIZE].try_into().unwrap()) as usize;
    let end = LEN_PREFIX_SIZE + declared;
    if end > padded.len() {
        return Err(ProtoError::PaddedLengthInvalid {
            declared,
            actual: padded.len(),
        });
    }
    Ok(padded[LEN_PREFIX_SIZE..end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn empty_input_pads_to_smallest_bucket() {
        let padded = pad(&[]).unwrap();
        assert_eq!(padded.len(), BUCKETS[0]);
    }

    #[test]
    fn input_too_large_for_any_bucket_errors() {
        let data = vec![0u8; BUCKETS[2] + 1];
        assert!(matches!(pad(&data), Err(ProtoError::TooLargeToPad { .. })));
    }

    #[test]
    fn unpad_rejects_short_buffer() {
        assert_eq!(unpad(&[0, 1]), Err(ProtoError::PaddedBufferTooShort));
    }

    #[test]
    fn unpad_rejects_inflated_length_prefix() {
        // Declares a length far larger than the buffer actually holds.
        let mut buf = vec![0xFFu8; 4];
        buf.extend_from_slice(&[1, 2, 3]);
        assert!(matches!(
            unpad(&buf),
            Err(ProtoError::PaddedLengthInvalid { .. })
        ));
    }

    proptest! {
        #[test]
        fn pad_then_unpad_roundtrips(data in proptest::collection::vec(any::<u8>(), 0..=4000)) {
            let padded = pad(&data).unwrap();
            let recovered = unpad(&padded).unwrap();
            prop_assert_eq!(recovered, data);
        }

        #[test]
        fn padded_length_is_always_a_known_bucket(data in proptest::collection::vec(any::<u8>(), 0..=4000)) {
            let padded = pad(&data).unwrap();
            prop_assert!(BUCKETS.contains(&padded.len()));
        }

        #[test]
        fn padded_length_never_reveals_exact_input_size(
            a in proptest::collection::vec(any::<u8>(), 0..=250),
            b in proptest::collection::vec(any::<u8>(), 0..=250),
        ) {
            // Two different small inputs land in the same bucket, so their
            // padded lengths are indistinguishable from each other.
            let padded_a = pad(&a).unwrap();
            let padded_b = pad(&b).unwrap();
            prop_assert_eq!(padded_a.len(), padded_b.len());
        }
    }
}
