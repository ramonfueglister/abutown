use abutown_protocol::DirectionDto;

const CHUNK_SIZE_F: f32 = 32.0;

#[inline]
fn chunk_center(cx: i32, cy: i32) -> (f32, f32) {
    (
        cx as f32 * CHUNK_SIZE_F + CHUNK_SIZE_F / 2.0,
        cy as f32 * CHUNK_SIZE_F + CHUNK_SIZE_F / 2.0,
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkGeometry {
    pub points: Vec<(f32, f32)>,
}

/// Computes the world coordinate at `progress` along the given polyline slice.
/// Zero allocations — operates on the slice directly.
pub fn world_coord_at_progress_slice(points: &[(f32, f32)], progress: f32) -> (f32, f32) {
    if points.len() < 2 {
        return points.first().copied().unwrap_or((0.0, 0.0));
    }
    let t = progress.clamp(0.0, 1.0);
    let total = arc_length_slice(points);
    if total <= 0.0 {
        return points[0];
    }
    let target = t * total;
    let mut walked = 0.0;
    for window in points.windows(2) {
        let (ax, ay) = window[0];
        let (bx, by) = window[1];
        let seg = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        if walked + seg >= target {
            let local_t = if seg > 0.0 {
                (target - walked) / seg
            } else {
                0.0
            };
            return (ax + (bx - ax) * local_t, ay + (by - ay) * local_t);
        }
        walked += seg;
    }
    *points.last().unwrap()
}

/// Computes the facing direction at `progress` along the given polyline slice.
/// Zero allocations — operates on the slice directly.
pub fn direction_at_progress_slice(
    points: &[(f32, f32)],
    progress: f32,
) -> abutown_protocol::DirectionDto {
    if points.len() < 2 {
        return abutown_protocol::DirectionDto::S;
    }
    let t = progress.clamp(0.0, 1.0);
    let total = arc_length_slice(points);
    if total <= 0.0 {
        return abutown_protocol::DirectionDto::S;
    }
    let target = t * total;
    let mut walked = 0.0;
    for window in points.windows(2) {
        let (ax, ay) = window[0];
        let (bx, by) = window[1];
        let seg = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        if walked + seg >= target {
            return direction_from_delta(bx - ax, by - ay);
        }
        walked += seg;
    }
    let (ax, ay) = points[points.len() - 2];
    let (bx, by) = *points.last().unwrap();
    direction_from_delta(bx - ax, by - ay)
}

fn arc_length_slice(points: &[(f32, f32)]) -> f32 {
    let mut total = 0.0;
    for window in points.windows(2) {
        let (ax, ay) = window[0];
        let (bx, by) = window[1];
        total += ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
    }
    total
}

impl LinkGeometry {
    pub fn world_coord_at_progress(&self, progress: f32) -> (f32, f32) {
        world_coord_at_progress_slice(&self.points, progress)
    }

    pub fn direction_at_progress(&self, progress: f32) -> abutown_protocol::DirectionDto {
        direction_at_progress_slice(&self.points, progress)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StopGeometry {
    pub coord: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivityGeometry {
    pub coord: (f32, f32),
}

pub fn link_geometry(link_id: &str) -> Option<LinkGeometry> {
    match link_id {
        "link:horizontal:main" => Some(LinkGeometry {
            points: vec![chunk_center(4, 4), chunk_center(5, 4)],
        }),
        "link:vertical:main" => Some(LinkGeometry {
            points: vec![chunk_center(4, 4), chunk_center(4, 5)],
        }),
        "link:walk:default" => Some(LinkGeometry {
            points: vec![chunk_center(4, 4), chunk_center(5, 4)],
        }),
        _ => None,
    }
}

pub fn stop_geometry(stop_id: &str) -> Option<StopGeometry> {
    match stop_id {
        "stop:horizontal:pickup" => Some(StopGeometry {
            coord: chunk_center(4, 4),
        }),
        "stop:horizontal:dropoff" => Some(StopGeometry {
            coord: chunk_center(5, 4),
        }),
        "stop:vertical:pickup" => Some(StopGeometry {
            coord: chunk_center(4, 4),
        }),
        "stop:vertical:dropoff" => Some(StopGeometry {
            coord: chunk_center(4, 5),
        }),
        _ => None,
    }
}

pub fn activity_geometry(activity_id: &str) -> Option<ActivityGeometry> {
    match activity_id {
        "activity:work" => Some(ActivityGeometry {
            coord: chunk_center(5, 4),
        }),
        _ => Some(ActivityGeometry {
            coord: chunk_center(4, 4),
        }),
    }
}

/// Returns the world coordinate along a route at `(link_index, progress)`.
/// Used when computing transit-vehicle positions.
pub fn route_link_world_coord(
    route_id: &str,
    link_index: usize,
    progress: f32,
) -> Option<(f32, f32)> {
    let link_id = match (route_id, link_index) {
        ("route:horizontal", 0) => "link:horizontal:main",
        ("route:vertical", 0) => "link:vertical:main",
        _ => return None,
    };
    let geom = link_geometry(link_id)?;
    Some(geom.world_coord_at_progress(progress))
}

/// Maps a unit-ish movement delta to the closest 8-way direction.
/// `(0,0)` returns `S` as a stable default for stationary entities.
pub fn direction_from_delta(dx: f32, dy: f32) -> DirectionDto {
    if dx == 0.0 && dy == 0.0 {
        return DirectionDto::S;
    }
    let angle = dy.atan2(dx); // -PI..PI, with E = 0, S = PI/2, W = ±PI, N = -PI/2
    let sector = ((angle / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8);
    match sector {
        0 => DirectionDto::E,
        1 => DirectionDto::Se,
        2 => DirectionDto::S,
        3 => DirectionDto::Sw,
        4 => DirectionDto::W,
        5 => DirectionDto::Nw,
        6 => DirectionDto::N,
        7 => DirectionDto::Ne,
        _ => DirectionDto::S,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_geometry_lookup_returns_seeded_routes() {
        let h = link_geometry("link:horizontal:main").expect("horizontal link defined");
        assert_eq!(
            h.points.first(),
            Some(&(4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0))
        );
        assert_eq!(
            h.points.last(),
            Some(&(5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0))
        );
        assert_eq!(h.points.len(), 2);

        let v = link_geometry("link:vertical:main").expect("vertical link defined");
        assert_eq!(
            v.points.first(),
            Some(&(4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0))
        );
        assert_eq!(
            v.points.last(),
            Some(&(4.0 * 32.0 + 16.0, 5.0 * 32.0 + 16.0))
        );
        assert_eq!(v.points.len(), 2);

        assert!(
            link_geometry("link:walk:default").is_some(),
            "walk link must be defined for seeded agents"
        );
    }

    #[test]
    fn stop_geometry_lookup_returns_seeded_stops() {
        let pickup = stop_geometry("stop:horizontal:pickup").expect("horizontal pickup defined");
        assert_eq!(pickup.coord, (4.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
        let dropoff = stop_geometry("stop:horizontal:dropoff").expect("horizontal dropoff defined");
        assert_eq!(dropoff.coord, (5.0 * 32.0 + 16.0, 4.0 * 32.0 + 16.0));
    }

    #[test]
    fn activity_geometry_falls_back_to_default_when_unknown() {
        let known = activity_geometry("activity:work").expect("work activity defined");
        assert!(known.coord.0 >= 0.0);
        assert!(
            activity_geometry("activity:unknown").is_some(),
            "unknown activities must still resolve to a default coord"
        );
    }

    #[test]
    fn route_link_geometry_interpolates_progress() {
        let coord = route_link_world_coord("route:horizontal", 0, 0.5).expect("route exists");
        assert!((coord.0 - (4.0 * 32.0 + 16.0 + 16.0)).abs() < 0.01);
        assert!((coord.1 - (4.0 * 32.0 + 16.0)).abs() < 0.01);
    }

    #[test]
    fn direction_from_delta_matches_compass() {
        use abutown_protocol::DirectionDto;
        assert_eq!(direction_from_delta(1.0, 0.0), DirectionDto::E);
        assert_eq!(direction_from_delta(0.0, -1.0), DirectionDto::N);
        assert_eq!(direction_from_delta(-1.0, 0.0), DirectionDto::W);
        assert_eq!(direction_from_delta(0.0, 1.0), DirectionDto::S);
        assert_eq!(direction_from_delta(1.0, 1.0), DirectionDto::Se);
        assert_eq!(direction_from_delta(0.0, 0.0), DirectionDto::S);
    }

    #[test]
    fn polyline_world_coord_at_progress_walks_arc_length() {
        let geom = LinkGeometry {
            points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
        };
        // Total arc length = 20. At progress 0.25, we're 5 units in (along first segment).
        assert_eq!(geom.world_coord_at_progress(0.0), (0.0, 0.0));
        let mid_first = geom.world_coord_at_progress(0.25);
        assert!((mid_first.0 - 5.0).abs() < 0.01);
        assert!((mid_first.1 - 0.0).abs() < 0.01);

        // At progress 0.75 we're 15 units in: full first segment (10) + 5 on second.
        let mid_second = geom.world_coord_at_progress(0.75);
        assert!((mid_second.0 - 10.0).abs() < 0.01);
        assert!((mid_second.1 - 5.0).abs() < 0.01);

        let end = geom.world_coord_at_progress(1.0);
        assert!((end.0 - 10.0).abs() < 0.01);
        assert!((end.1 - 10.0).abs() < 0.01);
    }

    #[test]
    fn polyline_direction_at_progress_returns_local_segment_direction() {
        use abutown_protocol::DirectionDto;
        let geom = LinkGeometry {
            points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, -10.0)],
        };
        assert_eq!(geom.direction_at_progress(0.25), DirectionDto::E);
        assert_eq!(geom.direction_at_progress(0.75), DirectionDto::N);
    }

    #[test]
    fn polyline_with_two_points_matches_old_start_end_semantics() {
        let geom = LinkGeometry {
            points: vec![(0.0, 0.0), (10.0, 0.0)],
        };
        assert_eq!(geom.world_coord_at_progress(0.5), (5.0, 0.0));
    }
}
