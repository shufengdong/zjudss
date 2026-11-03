use super::NumericPoint;

pub fn lttb(points: &[NumericPoint], threshold: usize) -> Vec<NumericPoint> {
    if threshold == 0 {
        return Vec::new();
    }
    if points.len() <= threshold || threshold < 3 {
        return points.to_vec();
    }

    let mut sampled = Vec::with_capacity(threshold);
    sampled.push(points[0].clone());
    let every = (points.len() - 2) as f64 / (threshold - 2) as f64;
    let mut selected = 0usize;

    for bucket in 0..(threshold - 2) {
        let average_start =
            (((bucket + 1) as f64 * every).floor() as usize + 1).min(points.len() - 1);
        let average_end = (((bucket + 2) as f64 * every).floor() as usize + 1).min(points.len());
        let average_slice = &points[average_start..average_end];
        let (average_x, average_y) = if average_slice.is_empty() {
            let point = &points[points.len() - 1];
            (point.x, point.y)
        } else {
            let count = average_slice.len() as f64;
            (
                average_slice.iter().map(|point| point.x).sum::<f64>() / count,
                average_slice.iter().map(|point| point.y).sum::<f64>() / count,
            )
        };

        let range_start = ((bucket as f64 * every).floor() as usize + 1).min(points.len() - 1);
        let range_end = (((bucket + 1) as f64 * every).floor() as usize + 1).min(points.len() - 1);
        let anchor = &points[selected];
        let mut best = range_start;
        let mut best_area = -1.0f64;
        for index in range_start..range_end.max(range_start + 1) {
            let point = &points[index];
            let area = ((anchor.x - average_x) * (point.y - anchor.y)
                - (anchor.x - point.x) * (average_y - anchor.y))
                .abs();
            if area > best_area || (area == best_area && point.ordinal < points[best].ordinal) {
                best_area = area;
                best = index;
            }
        }
        sampled.push(points[best].clone());
        selected = best;
    }

    sampled.push(points[points.len() - 1].clone());
    sampled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::charting::NumericPoint;

    #[test]
    fn lttb_keeps_endpoints_and_is_deterministic_for_equal_x_values() {
        let points = vec![
            NumericPoint {
                x: 0.0,
                y: 0.0,
                ordinal: 0,
            },
            NumericPoint {
                x: 1.0,
                y: 3.0,
                ordinal: 1,
            },
            NumericPoint {
                x: 1.0,
                y: 1.0,
                ordinal: 2,
            },
            NumericPoint {
                x: 2.0,
                y: 4.0,
                ordinal: 3,
            },
            NumericPoint {
                x: 3.0,
                y: 0.0,
                ordinal: 4,
            },
        ];

        let sampled = lttb(&points, 3);

        assert_eq!(sampled.len(), 3);
        assert_eq!(sampled.first().unwrap().ordinal, 0);
        assert_eq!(sampled.last().unwrap().ordinal, 4);
        assert_eq!(sampled, lttb(&points, 3));
    }
}
