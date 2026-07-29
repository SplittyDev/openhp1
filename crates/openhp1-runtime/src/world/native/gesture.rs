use super::*;

pub(super) fn gesture_points(
    runtime: &mut ScriptRuntime,
    class: &ResolvedObject,
    instance: &InstanceState,
) -> std::result::Result<Vec<[f32; 3]>, String> {
    let Some(StoredValue::Array(points)) = runtime
        .instance_property(class, instance, "Points")
        .map_err(|error| error.to_string())?
    else {
        return Ok(Vec::new());
    };
    Ok(points
        .into_iter()
        .filter_map(|point| match point {
            StoredValue::Value(Value::Vector(point)) => Some(point),
            _ => None,
        })
        .collect())
}

pub(super) fn gesture_native(
    index: u16,
    arguments: &[Value],
    points: &[[f32; 3]],
) -> std::result::Result<Value, String> {
    match (index, arguments) {
        (COMPARE_GESTURE, [Value::Array(input), Value::Float(accuracy)]) => {
            let mut drawn: Vec<[f32; 2]> = Vec::new();
            for point in input.iter().take(1024) {
                let Value::Vector([x, y, z]) = point else {
                    return Err("CompareGesture input array contains a non-vector".to_owned());
                };
                if *z == -1.0
                    || drawn.iter().any(|[old_x, old_y]| {
                        f32::abs(*old_x - x) < 0.001 && f32::abs(*old_y - y) < 0.001
                    })
                {
                    continue;
                }
                drawn.push([*x, *y]);
            }
            Ok(Value::Float(compare_gesture(
                &drawn,
                &resample_gesture(points),
                *accuracy,
            )))
        }
        (COMPARE_GESTURE_POINT, [Value::Vector(point), Value::Float(_)]) => Ok(Value::Float(
            nearest_gesture_distance([point[0], point[1]], &resample_gesture(points)),
        )),
        (COMPARE_GESTURE, _) => Err(format!(
            "CompareGesture expects a vector array and float, found ({})",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        (COMPARE_GESTURE_POINT, _) => Err(format!(
            "CompareGesturePoint expects a vector and float, found ({})",
            arguments
                .iter()
                .map(Value::kind)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => unreachable!(),
    }
}

fn resample_gesture(points: &[[f32; 3]]) -> Vec<[f32; 2]> {
    let points = &points[..points.len().min(1024)];
    let mut sampled = Vec::with_capacity(points.len().saturating_mul(8).saturating_sub(7));
    for (index, point) in points.iter().enumerate() {
        sampled.push([point[0], point[1]]);
        let Some(next) = points.get(index + 1) else {
            break;
        };
        for step in 1..8 {
            let fraction = step as f32 / 8.0;
            sampled.push([
                point[0] + (next[0] - point[0]) * fraction,
                point[1] + (next[1] - point[1]) * fraction,
            ]);
        }
    }
    sampled
}

fn nearest_gesture_distance(point: [f32; 2], points: &[[f32; 2]]) -> f32 {
    points.iter().fold(1.0, |distance, candidate| {
        distance.min(((point[0] - candidate[0]).powi(2) + (point[1] - candidate[1]).powi(2)).sqrt())
    })
}

fn compare_gesture(drawn: &[[f32; 2]], template: &[[f32; 2]], accuracy: f32) -> f32 {
    let mut penalty = 0;
    let mut total = 0;
    for &point in template {
        let distance = nearest_gesture_distance(point, drawn);
        if distance > accuracy {
            let weight = (distance / (accuracy * 5.0) + 1.0).round().min(2.0) as i32;
            penalty += weight;
            total += weight;
        } else {
            total += 2;
        }
    }
    for &point in drawn {
        let distance = nearest_gesture_distance(point, template);
        if distance > accuracy {
            let weight = (distance / accuracy).round().min(3.0) as i32;
            penalty += weight;
            total += weight;
        }
    }
    if drawn.len() < 10 {
        penalty += 500;
        total += 500;
    }
    if total == 0 {
        return 0.0;
    }
    (1.0 - penalty as f32 / total as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod gesture_tests {
    use super::*;

    #[test]
    fn gesture_comparison_matches_the_original_resampling_and_coverage_score() {
        let template = resample_gesture(&[[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [1.0, 0.0, 0.0]]);
        assert_eq!(template.len(), 17);
        assert_eq!(nearest_gesture_distance([0.5, 0.25], &template), 0.25);
        assert_eq!(compare_gesture(&template, &template, 0.1), 1.0);
        assert!(compare_gesture(&template[..4], &template, 0.1) < 1.0);
        assert_eq!(
            compare_gesture(
                &template
                    .iter()
                    .map(|[x, y]| [x + 2.0, y + 2.0])
                    .collect::<Vec<_>>(),
                &template,
                0.1,
            ),
            0.0
        );
    }
}
