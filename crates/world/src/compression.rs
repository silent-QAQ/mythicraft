use std::io::Read;

use flate2::bufread::GzDecoder;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GzipDecodeError {
    Decode(String),
    TooLarge { max_bytes: usize },
    TrailingData { trailing_bytes: usize },
}

pub(crate) fn decode_gzip_limited(
    payload: &[u8],
    max_bytes: usize,
) -> Result<Vec<u8>, GzipDecodeError> {
    let mut decoder = GzDecoder::new(payload);
    let read_limit = max_bytes.saturating_add(1) as u64;
    let mut output = Vec::new();
    (&mut decoder)
        .take(read_limit)
        .read_to_end(&mut output)
        .map_err(|error| GzipDecodeError::Decode(error.to_string()))?;
    if output.len() > max_bytes {
        return Err(GzipDecodeError::TooLarge { max_bytes });
    }
    let trailing_bytes = decoder.get_ref().len();
    if trailing_bytes != 0 {
        return Err(GzipDecodeError::TrailingData { trailing_bytes });
    }
    Ok(output)
}
