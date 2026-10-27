use std::collections::VecDeque;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::adapters::{AdapterError, CapturedStream, ErrorCategory};

pub(crate) async fn capture_stream(
    mut reader: impl AsyncRead + Unpin,
    retained_limit: usize,
) -> Result<CapturedStream, AdapterError> {
    let head_limit = retained_limit / 2;
    let tail_limit = retained_limit.saturating_sub(head_limit);
    let mut head = Vec::with_capacity(head_limit);
    let mut tail = VecDeque::with_capacity(tail_limit);
    let mut hasher = Sha256::new();
    let mut total_bytes = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let count = reader.read(&mut buffer).await.map_err(|error| {
            AdapterError::new(
                "engine_output_read_failed",
                ErrorCategory::Io,
                error.to_string(),
            )
        })?;
        if count == 0 {
            break;
        }
        let bytes = &buffer[..count];
        total_bytes = total_bytes.saturating_add(count as u64);
        hasher.update(bytes);
        for byte in bytes {
            if head.len() < head_limit {
                head.push(*byte);
            } else if tail_limit > 0 {
                if tail.len() == tail_limit {
                    tail.pop_front();
                }
                tail.push_back(*byte);
            }
        }
    }

    let mut retained = head;
    retained.extend(tail);
    let retained_sha256 = hex::encode(Sha256::digest(&retained));
    Ok(CapturedStream {
        truncated: total_bytes > retained.len() as u64,
        retained,
        full_sha256: hex::encode(hasher.finalize()),
        retained_sha256,
        total_bytes,
    })
}
