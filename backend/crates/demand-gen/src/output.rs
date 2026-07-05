//! `trips.bin` writer — the byte-stable binary trip table consumed by
//! `winterthur-traffic` (Task 5's loader is the read side of this format).
//!
//! Layout (all little-endian):
//!   header: magic u32 = 0x54524950 ("TRIP"), version u16 = 1,
//!           net_hash [u8;32] (blake3 of the trafficnet.json bytes),
//!           weekday_count u32, weekend_count u32
//!   body:   weekday block then weekend block, each block sorted by
//!           (departure_s, origin_lane, dest_lane); record = departure_s u32,
//!           origin_lane u32, dest_lane u32, segment u8, vehicle_class u8
//!           = 14 bytes.

use std::io::{self, Write};

pub const MAGIC: u32 = 0x5452_4950; // "TRIP" when read as LE bytes "PIRT"… see tests
pub const VERSION: u16 = 1;
pub const RECORD_BYTES: usize = 14;
pub const HEADER_BYTES: usize = 4 + 2 + 32 + 4 + 4;

/// Trip segment codes (`TripRecord::segment`).
pub const SEGMENT_INTERNAL: u8 = 0;
pub const SEGMENT_INBOUND: u8 = 1;
pub const SEGMENT_OUTBOUND: u8 = 2;
pub const SEGMENT_THROUGH: u8 = 3;

/// One scheduled trip. `vehicle_class` is always 0 (car) in version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TripRecord {
    pub departure_s: u32,
    pub origin_lane: u32,
    pub dest_lane: u32,
    pub segment: u8,
    pub vehicle_class: u8,
}

impl TripRecord {
    /// The 14-byte little-endian wire form.
    pub fn encode(&self) -> [u8; RECORD_BYTES] {
        let mut b = [0u8; RECORD_BYTES];
        b[0..4].copy_from_slice(&self.departure_s.to_le_bytes());
        b[4..8].copy_from_slice(&self.origin_lane.to_le_bytes());
        b[8..12].copy_from_slice(&self.dest_lane.to_le_bytes());
        b[12] = self.segment;
        b[13] = self.vehicle_class;
        b
    }

    /// Inverse of [`TripRecord::encode`].
    pub fn decode(b: &[u8; RECORD_BYTES]) -> Self {
        TripRecord {
            departure_s: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            origin_lane: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            dest_lane: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            segment: b[12],
            vehicle_class: b[13],
        }
    }

    /// The in-block sort key mandated by the format.
    pub fn sort_key(&self) -> (u32, u32, u32) {
        (self.departure_s, self.origin_lane, self.dest_lane)
    }
}

/// Write a complete `trips.bin` stream: header + sorted weekday block +
/// sorted weekend block. The input slices need not be pre-sorted — this
/// function sorts (stably, by [`TripRecord::sort_key`]) so the output is
/// byte-stable for any input order.
pub fn write_trips<W: Write>(
    w: &mut W,
    net_hash: &[u8; 32],
    weekday: &[TripRecord],
    weekend: &[TripRecord],
) -> io::Result<()> {
    let mut weekday = weekday.to_vec();
    let mut weekend = weekend.to_vec();
    weekday.sort_by_key(TripRecord::sort_key);
    weekend.sort_by_key(TripRecord::sort_key);

    w.write_all(&MAGIC.to_le_bytes())?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(net_hash)?;
    w.write_all(
        &u32::try_from(weekday.len())
            .expect("weekday count fits u32")
            .to_le_bytes(),
    )?;
    w.write_all(
        &u32::try_from(weekend.len())
            .expect("weekend count fits u32")
            .to_le_bytes(),
    )?;
    for r in weekday.iter().chain(weekend.iter()) {
        w.write_all(&r.encode())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(dep: u32, o: u32, d: u32, seg: u8) -> TripRecord {
        TripRecord {
            departure_s: dep,
            origin_lane: o,
            dest_lane: d,
            segment: seg,
            vehicle_class: 0,
        }
    }

    #[test]
    fn record_encode_decode_round_trip() {
        let r = rec(27_000, 123_456, 7, SEGMENT_THROUGH);
        let bytes = r.encode();
        assert_eq!(bytes.len(), RECORD_BYTES);
        assert_eq!(TripRecord::decode(&bytes), r);
        // spot-check the LE layout: departure_s first
        assert_eq!(&bytes[0..4], &27_000u32.to_le_bytes());
        assert_eq!(bytes[12], SEGMENT_THROUGH);
        assert_eq!(bytes[13], 0);
    }

    #[test]
    fn write_trips_header_and_sorted_blocks() {
        let net_hash = [0xABu8; 32];
        // deliberately unsorted input
        let weekday = vec![
            rec(200, 5, 9, SEGMENT_INTERNAL),
            rec(100, 8, 1, SEGMENT_INBOUND),
            rec(100, 3, 2, SEGMENT_OUTBOUND),
            rec(100, 3, 1, SEGMENT_OUTBOUND),
        ];
        let weekend = vec![rec(50, 2, 2, SEGMENT_THROUGH)];

        let mut buf = Vec::new();
        write_trips(&mut buf, &net_hash, &weekday, &weekend).unwrap();

        assert_eq!(buf.len(), HEADER_BYTES + 5 * RECORD_BYTES);
        assert_eq!(&buf[0..4], &MAGIC.to_le_bytes());
        assert_eq!(&buf[4..6], &VERSION.to_le_bytes());
        assert_eq!(&buf[6..38], &net_hash[..]);
        assert_eq!(&buf[38..42], &4u32.to_le_bytes());
        assert_eq!(&buf[42..46], &1u32.to_le_bytes());

        let records: Vec<TripRecord> = buf[HEADER_BYTES..]
            .chunks_exact(RECORD_BYTES)
            .map(|c| TripRecord::decode(c.try_into().unwrap()))
            .collect();
        // weekday block sorted by (departure_s, origin_lane, dest_lane)
        let wd = &records[..4];
        assert_eq!(wd[0], rec(100, 3, 1, SEGMENT_OUTBOUND));
        assert_eq!(wd[1], rec(100, 3, 2, SEGMENT_OUTBOUND));
        assert_eq!(wd[2], rec(100, 8, 1, SEGMENT_INBOUND));
        assert_eq!(wd[3], rec(200, 5, 9, SEGMENT_INTERNAL));
        for w in wd.windows(2) {
            assert!(
                w[0].sort_key() <= w[1].sort_key(),
                "weekday block not sorted"
            );
        }
        // weekend block follows
        assert_eq!(records[4], rec(50, 2, 2, SEGMENT_THROUGH));
    }

    #[test]
    fn write_trips_is_input_order_independent() {
        let net_hash = [1u8; 32];
        let a = vec![rec(1, 1, 1, 0), rec(2, 2, 2, 0), rec(3, 3, 3, 0)];
        let mut b = a.clone();
        b.reverse();
        let mut out_a = Vec::new();
        let mut out_b = Vec::new();
        write_trips(&mut out_a, &net_hash, &a, &[]).unwrap();
        write_trips(&mut out_b, &net_hash, &b, &[]).unwrap();
        assert_eq!(out_a, out_b);
    }
}
