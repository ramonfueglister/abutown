//! `trips.bin` loader — the read side of the binary trip table written by
//! the offline `demand-gen` tool (`demand_gen::output` is the write side).
//!
//! The loader is deliberately independent of the `demand-gen` crate: the
//! format constants are restated here and cross-checked in tests, which
//! build fixture files through the real writer (dev-dependency only).
//!
//! Layout (all little-endian):
//!   header (46 B): magic u32 = 0x54524950, version u16 = 1,
//!                  net_hash [u8;32] (blake3 of the trafficnet.json bytes),
//!                  weekday_count u32, weekend_count u32
//!   body: weekday block then weekend block, each sorted by
//!         (departure_s, origin_lane, dest_lane); record (14 B) =
//!         departure_s u32, origin_lane u32, dest_lane u32,
//!         segment u8, vehicle_class u8.
//!
//! Any structural violation (magic, version, net-hash mismatch, truncation,
//! unsorted block) is a hard [`DemandError`] — no healing, no fallback,
//! mirroring `traffic_net::NetError`.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Expected `trips.bin` magic ("TRIP" interpreted as a LE u32).
const MAGIC: u32 = 0x5452_4950;
/// The only format version this loader understands.
const VERSION: u16 = 1;
/// Header size in bytes: magic + version + net_hash + two counts.
const HEADER_BYTES: usize = 4 + 2 + 32 + 4 + 4;
/// Record size in bytes.
const RECORD_BYTES: usize = 14;

/// Day class the schedule distinguishes (weekday block vs weekend block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayKind {
    Workday,
    Weekend,
}

/// One scheduled trip, decoded from its 14-byte record.
///
/// `index` is the record's position within its block (weekday and weekend
/// blocks each start at 0) — the stable per-trip identity used for
/// deterministic thinning by the spawner. `vehicle_class` indexes the
/// kernel's per-class IDM table (see [`traffic_core::idm::N_CLASSES`]); an
/// out-of-range class in the file is a hard load error, not a healed car.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trip {
    pub departure_s: u32,
    pub origin_lane: u32,
    pub dest_lane: u32,
    pub segment: u8,
    pub vehicle_class: u8,
    pub index: u32,
}

/// Hard load failures. Every variant is descriptive and terminal — a bad
/// `trips.bin` must never half-load.
#[derive(Debug, Error)]
pub enum DemandError {
    #[error("failed to read trips file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("trips file {path} is truncated: {actual} bytes, need at least {expected}")]
    Truncated {
        path: PathBuf,
        expected: usize,
        actual: usize,
    },

    #[error("trips file {path} has bad magic {found:#010x}, expected {MAGIC:#010x}")]
    BadMagic { path: PathBuf, found: u32 },

    #[error("trips file {path} has unsupported version {found}, expected {VERSION}")]
    BadVersion { path: PathBuf, found: u16 },

    #[error(
        "trips file {path} net_hash mismatch: file was baked against net blake3 {file_hash}, \
         but the loaded trafficnet.json hashes to {net_hash} — re-run demand-gen"
    )]
    NetHashMismatch {
        path: PathBuf,
        file_hash: String,
        net_hash: String,
    },

    #[error(
        "trips file {path} body length mismatch: header declares {declared} records \
         ({expected} body bytes) but the body is {actual} bytes"
    )]
    BodyLengthMismatch {
        path: PathBuf,
        declared: u64,
        expected: u64,
        actual: usize,
    },

    #[error(
        "trips file {path} {day:?} block is not sorted by departure_s at record {index} \
         ({prev} > {next})"
    )]
    UnsortedBlock {
        path: PathBuf,
        day: DayKind,
        index: u32,
        prev: u32,
        next: u32,
    },
    #[error(
        "trips file {path} {day:?} block record {index} carries vehicle class {class}, \
         outside the kernel's class table (0..{})",
        traffic_core::idm::N_CLASSES
    )]
    BadVehicleClass {
        path: PathBuf,
        day: DayKind,
        index: u32,
        class: u8,
    },
}

/// The full authored trip table for one network, loaded and validated.
#[derive(Debug)]
pub struct TripSchedule {
    weekday: Vec<Trip>,
    weekend: Vec<Trip>,
}

impl TripSchedule {
    /// Load and validate `trips.bin` from `path`. `net_json_bytes` are the
    /// bytes of the `trafficnet.json` actually loaded by the server; their
    /// blake3 hash must equal the `net_hash` baked into the file header,
    /// otherwise the trip table was generated against a different network
    /// and loading fails hard.
    pub fn load(path: &Path, net_json_bytes: &[u8]) -> Result<TripSchedule, DemandError> {
        let bytes = std::fs::read(path).map_err(|source| DemandError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if bytes.len() < HEADER_BYTES {
            return Err(DemandError::Truncated {
                path: path.to_path_buf(),
                expected: HEADER_BYTES,
                actual: bytes.len(),
            });
        }

        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        if magic != MAGIC {
            return Err(DemandError::BadMagic {
                path: path.to_path_buf(),
                found: magic,
            });
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if version != VERSION {
            return Err(DemandError::BadVersion {
                path: path.to_path_buf(),
                found: version,
            });
        }

        let file_hash: [u8; 32] = bytes[6..38].try_into().unwrap();
        let net_hash = blake3::hash(net_json_bytes);
        if file_hash != *net_hash.as_bytes() {
            return Err(DemandError::NetHashMismatch {
                path: path.to_path_buf(),
                file_hash: blake3::Hash::from_bytes(file_hash).to_hex().to_string(),
                net_hash: net_hash.to_hex().to_string(),
            });
        }

        let weekday_count = u32::from_le_bytes(bytes[38..42].try_into().unwrap());
        let weekend_count = u32::from_le_bytes(bytes[42..46].try_into().unwrap());
        let declared = u64::from(weekday_count) + u64::from(weekend_count);
        let expected = declared * RECORD_BYTES as u64;
        let body = &bytes[HEADER_BYTES..];
        if body.len() as u64 != expected {
            return Err(DemandError::BodyLengthMismatch {
                path: path.to_path_buf(),
                declared,
                expected,
                actual: body.len(),
            });
        }

        let decode_block = |day: DayKind, records: &[u8]| -> Result<Vec<Trip>, DemandError> {
            let mut trips = Vec::with_capacity(records.len() / RECORD_BYTES);
            for (i, r) in records.chunks_exact(RECORD_BYTES).enumerate() {
                let index = u32::try_from(i).expect("block count fits u32 by header");
                let trip = Trip {
                    departure_s: u32::from_le_bytes(r[0..4].try_into().unwrap()),
                    origin_lane: u32::from_le_bytes(r[4..8].try_into().unwrap()),
                    dest_lane: u32::from_le_bytes(r[8..12].try_into().unwrap()),
                    segment: r[12],
                    vehicle_class: r[13],
                    index,
                };
                if trip.vehicle_class as usize >= traffic_core::idm::N_CLASSES {
                    return Err(DemandError::BadVehicleClass {
                        path: path.to_path_buf(),
                        day,
                        index,
                        class: trip.vehicle_class,
                    });
                }
                if let Some(prev) = trips.last().map(|p: &Trip| p.departure_s)
                    && prev > trip.departure_s
                {
                    return Err(DemandError::UnsortedBlock {
                        path: path.to_path_buf(),
                        day,
                        index,
                        prev,
                        next: trip.departure_s,
                    });
                }
                trips.push(trip);
            }
            Ok(trips)
        };

        let split = weekday_count as usize * RECORD_BYTES;
        Ok(TripSchedule {
            weekday: decode_block(DayKind::Workday, &body[..split])?,
            weekend: decode_block(DayKind::Weekend, &body[split..])?,
        })
    }

    /// All trips of `day` with `departure_s` in `[window.start, window.end)`,
    /// as a contiguous slice (binary search on the sorted block).
    ///
    /// Wrap handling is the CALLER's job: this function never wraps across
    /// midnight — a tick window spanning 86400 must be split into two calls
    /// (and the day kind re-evaluated). An empty or inverted range
    /// (`start >= end`) returns an empty slice.
    pub fn trips_in(&self, day: DayKind, window: core::ops::Range<u32>) -> &[Trip] {
        if window.start >= window.end {
            return &[];
        }
        let block = self.block(day);
        let lo = block.partition_point(|t| t.departure_s < window.start);
        let hi = block.partition_point(|t| t.departure_s < window.end);
        &block[lo..hi]
    }

    /// Number of trips in the block for `day`.
    pub fn count(&self, day: DayKind) -> usize {
        self.block(day).len()
    }

    fn block(&self, day: DayKind) -> &[Trip] {
        match day {
            DayKind::Workday => &self.weekday,
            DayKind::Weekend => &self.weekend,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use demand_gen::output::{self, TripRecord};
    use std::io::Write as _;

    const NET_JSON: &[u8] = br#"{"nodes":[],"edges":[],"lanes":[],"turns":[]}"#;

    fn rec(dep: u32, o: u32, d: u32, seg: u8) -> TripRecord {
        TripRecord {
            departure_s: dep,
            origin_lane: o,
            dest_lane: d,
            segment: seg,
            vehicle_class: 0,
        }
    }

    /// Write `bytes` to a unique temp file and return its path.
    fn temp_trips_file(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "winterthur-traffic-demand-test-{}-{name}.bin",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    /// Build a valid trips.bin (via the real demand-gen writer) whose
    /// net_hash matches `NET_JSON`.
    fn valid_trips_bytes(weekday: &[TripRecord], weekend: &[TripRecord]) -> Vec<u8> {
        let net_hash = *blake3::hash(NET_JSON).as_bytes();
        let mut buf = Vec::new();
        output::write_trips(&mut buf, &net_hash, weekday, weekend).unwrap();
        buf
    }

    #[test]
    fn load_ok_counts_and_indices() {
        let weekday = vec![
            rec(100, 7, 9, output::SEGMENT_INTERNAL),
            rec(200, 3, 4, output::SEGMENT_INBOUND),
            rec(300, 5, 6, output::SEGMENT_OUTBOUND),
        ];
        let weekend = vec![
            rec(50, 1, 2, output::SEGMENT_THROUGH),
            rec(60, 2, 1, output::SEGMENT_INTERNAL),
        ];
        let path = temp_trips_file("ok", &valid_trips_bytes(&weekday, &weekend));

        let sched = TripSchedule::load(&path, NET_JSON).unwrap();
        assert_eq!(sched.count(DayKind::Workday), 3);
        assert_eq!(sched.count(DayKind::Weekend), 2);

        let all = sched.trips_in(DayKind::Workday, 0..u32::MAX);
        assert_eq!(all.len(), 3);
        assert_eq!(
            all[0],
            Trip {
                departure_s: 100,
                origin_lane: 7,
                dest_lane: 9,
                segment: output::SEGMENT_INTERNAL,
                vehicle_class: 0,
                index: 0,
            }
        );
        assert_eq!(all[1].index, 1);
        assert_eq!(all[2].index, 2);

        // weekend block indices restart at 0
        let we = sched.trips_in(DayKind::Weekend, 0..u32::MAX);
        assert_eq!(we.len(), 2);
        assert_eq!(we[0].index, 0);
        assert_eq!(we[0].segment, output::SEGMENT_THROUGH);
        assert_eq!(we[1].index, 1);
    }

    #[test]
    fn load_rejects_wrong_net_bytes_mentioning_hash() {
        let path = temp_trips_file("wrong-net", &valid_trips_bytes(&[rec(1, 1, 1, 0)], &[]));
        let err = TripSchedule::load(&path, b"a different network").unwrap_err();
        assert!(matches!(err, DemandError::NetHashMismatch { .. }));
        let msg = err.to_string();
        assert!(
            msg.contains("net_hash"),
            "message must mention the hash: {msg}"
        );
        // both hex digests appear so the operator can diff them
        let net_hex = blake3::hash(b"a different network").to_hex().to_string();
        assert!(
            msg.contains(&net_hex),
            "message must contain the net hash: {msg}"
        );
    }

    #[test]
    fn load_rejects_corrupted_magic() {
        let mut bytes = valid_trips_bytes(&[rec(1, 1, 1, 0)], &[]);
        bytes[0] ^= 0xFF;
        let path = temp_trips_file("bad-magic", &bytes);
        let err = TripSchedule::load(&path, NET_JSON).unwrap_err();
        assert!(matches!(err, DemandError::BadMagic { .. }), "got {err:?}");
    }

    #[test]
    fn load_rejects_unknown_version() {
        let mut bytes = valid_trips_bytes(&[rec(1, 1, 1, 0)], &[]);
        bytes[4] = 0xFE; // version LE low byte
        let path = temp_trips_file("bad-version", &bytes);
        let err = TripSchedule::load(&path, NET_JSON).unwrap_err();
        assert!(matches!(err, DemandError::BadVersion { .. }), "got {err:?}");
    }

    #[test]
    fn load_rejects_truncated_body() {
        let mut bytes = valid_trips_bytes(&[rec(1, 1, 1, 0), rec(2, 2, 2, 0)], &[]);
        bytes.truncate(bytes.len() - 1);
        let path = temp_trips_file("truncated", &bytes);
        let err = TripSchedule::load(&path, NET_JSON).unwrap_err();
        assert!(
            matches!(err, DemandError::BodyLengthMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn load_accepts_known_classes_and_rejects_unknown() {
        // Classes 0..N_CLASSES load and survive the round trip.
        let mut van = rec(100, 1, 2, output::SEGMENT_INTERNAL);
        van.vehicle_class = 1;
        let mut truck = rec(200, 3, 4, output::SEGMENT_THROUGH);
        truck.vehicle_class = 2;
        let path = temp_trips_file("classes-ok", &valid_trips_bytes(&[van, truck], &[]));
        let sched = TripSchedule::load(&path, NET_JSON).unwrap();
        let all = sched.trips_in(DayKind::Workday, 0..u32::MAX);
        assert_eq!(all[0].vehicle_class, 1);
        assert_eq!(all[1].vehicle_class, 2);

        // An out-of-range class is a hard load error (no healing to car).
        let mut bogus = rec(100, 1, 2, output::SEGMENT_INTERNAL);
        bogus.vehicle_class = traffic_core::idm::N_CLASSES as u8;
        let path = temp_trips_file("classes-bad", &valid_trips_bytes(&[bogus], &[]));
        let err = TripSchedule::load(&path, NET_JSON).unwrap_err();
        assert!(
            matches!(err, DemandError::BadVehicleClass { class, .. } if class == 3),
            "got {err:?}"
        );
    }

    #[test]
    fn load_rejects_missing_file() {
        let path = std::env::temp_dir().join("winterthur-traffic-demand-test-does-not-exist.bin");
        let err = TripSchedule::load(&path, NET_JSON).unwrap_err();
        assert!(matches!(err, DemandError::Io { .. }), "got {err:?}");
    }

    #[test]
    fn trips_in_window_is_exact_and_end_exclusive() {
        // records straddling the [25200, 25260) window
        let weekday = vec![
            rec(25_199, 1, 2, 0), // just before → excluded
            rec(25_200, 3, 4, 0), // start inclusive
            rec(25_230, 5, 6, 0),
            rec(25_259, 7, 8, 0),  // last inside
            rec(25_260, 9, 10, 0), // end exclusive → excluded
        ];
        let path = temp_trips_file("window", &valid_trips_bytes(&weekday, &[]));
        let sched = TripSchedule::load(&path, NET_JSON).unwrap();

        let w = sched.trips_in(DayKind::Workday, 25_200..25_260);
        let deps: Vec<u32> = w.iter().map(|t| t.departure_s).collect();
        assert_eq!(deps, vec![25_200, 25_230, 25_259]);

        // empty window
        assert!(sched.trips_in(DayKind::Workday, 25_230..25_230).is_empty());
        // inverted window (caller handles wrap) → empty, no panic
        #[allow(clippy::reversed_empty_ranges)] // deliberately inverted: documents the contract
        let inverted = 25_260..25_200;
        assert!(sched.trips_in(DayKind::Workday, inverted).is_empty());
        // window with no records
        assert!(sched.trips_in(DayKind::Workday, 0..100).is_empty());
        // weekend block is empty entirely
        assert!(sched.trips_in(DayKind::Weekend, 0..86_400).is_empty());
    }
}
