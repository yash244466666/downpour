//! Segment planning.
//!
//! The planner is deliberately pure: given a size, range support, and a
//! connection budget, it returns either a single stream or a set of segments
//! that exactly tile `[0, total)`.

use downpour_types::SegmentSpec;

pub const MIN_SEGMENT_BYTES: u64 = 1024 * 1024;
pub const MAX_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
/// Below this, many connections cost more than they save.
pub const MIN_MULTI_CONNECTION_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_CONNECTIONS: u16 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferPlan {
    /// No byte ranges (or unknown size): one request, no resume checkpoints.
    SingleStream { total_bytes: Option<u64> },
    /// Range-capable transfer split across workers.
    Segmented(Vec<SegmentSpec>),
}

impl TransferPlan {
    #[must_use]
    pub fn total_bytes(&self) -> Option<u64> {
        match self {
            Self::SingleStream { total_bytes } => *total_bytes,
            Self::Segmented(segments) => segments.last().map(|s| s.end + 1),
        }
    }

    #[must_use]
    pub fn is_segmented(&self) -> bool {
        matches!(self, Self::Segmented(_))
    }
}

/// Plan a transfer.
///
/// `total_bytes` must come from a validated `Content-Range` total, never from a
/// server's `Accept-Ranges` claim. `requested_connections` is clamped to
/// `1..=32`.
#[must_use]
pub fn plan_transfer(
    total_bytes: Option<u64>,
    ranges_supported: bool,
    requested_connections: u16,
) -> TransferPlan {
    let connections = u64::from(requested_connections.clamp(1, MAX_CONNECTIONS));

    let Some(total) = total_bytes else {
        return TransferPlan::SingleStream { total_bytes: None };
    };

    if !ranges_supported || connections <= 1 || total < MIN_MULTI_CONNECTION_BYTES {
        return TransferPlan::SingleStream {
            total_bytes: Some(total),
        };
    }

    // Aim for ~2 segments per connection so early finishers can take slack,
    // bounded by the per-connection limit.
    let target = total.div_ceil(connections * 2);
    let seg_size = target.clamp(MIN_SEGMENT_BYTES, MAX_SEGMENT_BYTES);
    let count = total.div_ceil(seg_size).clamp(1, connections);

    let per_segment = total.div_ceil(count);
    let mut segments = Vec::with_capacity(count as usize);
    let mut start = 0_u64;
    let mut index = 0_u32;

    while start < total {
        let end = (start + per_segment).min(total) - 1;
        segments.push(SegmentSpec::new(index, start, end));
        start = end + 1;
        index += 1;
    }

    TransferPlan::Segmented(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_tiles(segments: &[SegmentSpec], total: u64) {
        assert!(!segments.is_empty());
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments.last().unwrap().end, total - 1);
        for pair in segments.windows(2) {
            assert_eq!(pair[0].end + 1, pair[1].start, "gap or overlap");
        }
        for (idx, segment) in segments.iter().enumerate() {
            assert_eq!(segment.index, idx as u32);
            assert!(segment.end >= segment.start);
            assert!(!segment.is_empty());
        }
    }

    #[test]
    fn small_file_is_single_stream() {
        let plan = plan_transfer(Some(512 * 1024), true, 16);
        assert_eq!(
            plan,
            TransferPlan::SingleStream {
                total_bytes: Some(512 * 1024)
            }
        );
    }

    #[test]
    fn no_ranges_is_single_stream() {
        let plan = plan_transfer(Some(500 * 1024 * 1024), false, 16);
        assert!(!plan.is_segmented());
    }

    #[test]
    fn unknown_size_is_single_stream() {
        let plan = plan_transfer(None, true, 16);
        assert_eq!(plan, TransferPlan::SingleStream { total_bytes: None });
    }

    #[test]
    fn zero_size_is_single_stream() {
        let plan = plan_transfer(Some(0), true, 8);
        assert_eq!(
            plan,
            TransferPlan::SingleStream {
                total_bytes: Some(0)
            }
        );
    }

    #[test]
    fn single_connection_is_single_stream() {
        let plan = plan_transfer(Some(100 * 1024 * 1024), true, 1);
        assert!(!plan.is_segmented());
    }

    #[test]
    fn splits_a_large_file() {
        let total = 100 * 1024 * 1024;
        let TransferPlan::Segmented(segments) = plan_transfer(Some(total), true, 8) else {
            panic!("expected segmented plan");
        };
        assert_tiles(&segments, total);
        assert!(segments.len() <= 8);
        assert!(segments.len() >= 2);
    }

    #[test]
    fn respects_connection_cap() {
        let total = 4 * 1024 * 1024 * 1024;
        let TransferPlan::Segmented(segments) = plan_transfer(Some(total), true, 200) else {
            panic!("expected segmented plan");
        };
        assert!(segments.len() <= u64::from(MAX_CONNECTIONS) as usize);
    }

    #[test]
    fn exact_minimum_is_segmented() {
        let plan = plan_transfer(Some(MIN_MULTI_CONNECTION_BYTES), true, 8);
        assert!(plan.is_segmented());
    }

    #[test]
    fn two_mib_with_32_connections_stays_within_bounds() {
        let total = MIN_MULTI_CONNECTION_BYTES;
        let TransferPlan::Segmented(segments) = plan_transfer(Some(total), true, 32) else {
            panic!("expected segmented plan");
        };
        assert_tiles(&segments, total);
        assert!(segments.len() <= 32);
        for segment in &segments {
            assert!(segment.len() <= MAX_SEGMENT_BYTES);
        }
    }

    proptest::proptest! {
        #[test]
        fn segments_always_tile(total in (MIN_MULTI_CONNECTION_BYTES..(8_u64 * 1024 * 1024 * 1024)), conns in 2_u16..=32_u16) {
            if let TransferPlan::Segmented(segments) = plan_transfer(Some(total), true, conns) {
                proptest::prop_assert!(!segments.is_empty());
                proptest::prop_assert_eq!(segments[0].start, 0);
                proptest::prop_assert_eq!(segments.last().unwrap().end, total - 1);
                for pair in segments.windows(2) {
                    proptest::prop_assert_eq!(pair[0].end + 1, pair[1].start);
                }
                proptest::prop_assert!(segments.len() <= usize::from(conns));
            } else {
                proptest::prop_assert!(conns <= 1 || total < MIN_MULTI_CONNECTION_BYTES);
            }
        }
    }
}
