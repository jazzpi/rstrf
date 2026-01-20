use ndarray::Array1;

// TODO: How can we implement this for f32 as well?
pub fn minmax(arr: &Array1<f64>) -> (f64, f64) {
    if arr.len() == 0 {
        (f64::NAN, f64::NAN)
    } else {
        arr.iter()
            .cloned()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), val| {
                (min.min(val), max.max(val))
            })
    }
}

pub fn to_index(value: f32, max: usize) -> usize {
    value.round().clamp(0.0, (max - 1) as f32) as usize
}
