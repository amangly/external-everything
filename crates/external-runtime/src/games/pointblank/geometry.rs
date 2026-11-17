use super::offsets::{BOX_HEIGHT_ESTIMATE, BOX_WIDTH_HEIGHT_RATIO};

/// Scaled, pivoted elimination from internal/frame_builder.h::Nonsingular.
/// Scaling avoids treating a reasonable near/far range as a zero determinant.
pub(super) fn nonsingular(matrix: &[f32; 16]) -> bool {
    let mut values = [[0.0_f64; 4]; 4];
    for row in 0..4 {
        let mut scale = 0.0_f64;
        for col in 0..4 {
            let value = matrix[row * 4 + col];
            if !value.is_finite() {
                return false;
            }
            scale = scale.max(f64::from(value).abs());
        }
        if scale == 0.0 {
            return false;
        }
        for col in 0..4 {
            values[row][col] = f64::from(matrix[row * 4 + col]) / scale;
        }
    }
    for col in 0..4 {
        let mut pivot = col;
        for row in (col + 1)..4 {
            if values[row][col].abs() > values[pivot][col].abs() {
                pivot = row;
            }
        }
        if values[pivot][col].abs() <= 1e-8 {
            return false;
        }
        values.swap(col, pivot);
        let pivot_row = values[col];
        for row in values.iter_mut().skip(col + 1) {
            let ratio = row[col] / pivot_row[col];
            for (value, pivot) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= ratio * pivot;
            }
        }
    }
    true
}

/// Includes the stronger C++ proof guards missing in the old Rust port:
/// sparse perspective layout, orthonormal view rotation, and nonsingularity.
pub(super) fn camera_matrices(
    view: &[f32; 16],
    projection: &[f32; 16],
    width: f32,
    height: f32,
) -> bool {
    if !width.is_finite()
        || !height.is_finite()
        || !(320.0..=8000.0).contains(&width)
        || !(200.0..=8000.0).contains(&height)
    {
        return false;
    }
    for i in 0..16 {
        if !view[i].is_finite()
            || !projection[i].is_finite()
            || view[i].abs() > 1e6
            || projection[i].abs() > 1e6
        {
            return false;
        }
        if !matches!(i, 0 | 5 | 10 | 11 | 14) && projection[i].abs() > 1e-4 {
            return false;
        }
    }
    if view[3].abs() > 1e-4
        || view[7].abs() > 1e-4
        || view[11].abs() > 1e-4
        || (view[15] - 1.0).abs() > 1e-4
    {
        return false;
    }
    for row in 0..3 {
        for other in row..3 {
            let dot: f32 = (0..3)
                .map(|k| view[row * 4 + k] * view[other * 4 + k])
                .sum();
            let expected = if row == other { 1.0 } else { 0.0 };
            if (dot - expected).abs() > 0.003 {
                return false;
            }
        }
    }
    projection[0].abs() >= 0.05
        && projection[5].abs() >= 0.05
        && (projection[11].abs() - 1.0).abs() <= 0.001
        && projection[14].abs() > 1e-6
        && nonsingular(projection)
        && (projection[5] / projection[0] - width / height).abs() <= 0.001 * (width / height)
}

pub(super) fn multiply(view: &[f32; 16], projection: &[f32; 16]) -> Option<[f32; 16]> {
    let mut product = [0.0; 16];
    for row in 0..4 {
        for col in 0..4 {
            let value = (0..4)
                .map(|k| view[row * 4 + k] * projection[k * 4 + col])
                .sum::<f32>();
            if !value.is_finite() {
                return None;
            }
            product[row * 4 + col] = value;
        }
    }
    Some(product)
}

/// The ledger stores row-major matrices for row vectors: [x,y,z,1] * V * P.
/// Unlike the old shared ABI, this product stays untransposed throughout.
pub(super) fn project(
    product: &[f32; 16],
    world: [f32; 3],
    width: f32,
    height: f32,
) -> Option<[f32; 2]> {
    if !world.iter().all(|v| v.is_finite())
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    let clip = |column| {
        world[0] * product[column]
            + world[1] * product[4 + column]
            + world[2] * product[8 + column]
            + product[12 + column]
    };
    let x = clip(0);
    let y = clip(1);
    let w = clip(3);
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || w <= 1e-5 {
        return None;
    }
    let pixel = [(x / w * 0.5 + 0.5) * width, (-y / w * 0.5 + 0.5) * height];
    pixel.iter().all(|v| v.is_finite()).then_some(pixel)
}

pub(super) fn rectangle(
    product: &[f32; 16],
    position: [f32; 3],
    width: f32,
    height: f32,
) -> Option<[f32; 4]> {
    let mut estimated_top = position;
    estimated_top[1] += BOX_HEIGHT_ESTIMATE;
    let top_point = project(product, estimated_top, width, height)?;
    let bottom_point = project(product, position, width, height)?;
    let top = top_point[1].min(bottom_point[1]);
    let bottom = top_point[1].max(bottom_point[1]);
    let box_height = bottom - top;
    if !(0.5..=10_000.0).contains(&box_height) {
        return None;
    }
    let center = (top_point[0] + bottom_point[0]) * 0.5;
    let box_width = (box_height * BOX_WIDTH_HEIGHT_RATIO).max(2.0);
    let left = center - box_width * 0.5;
    let right = center + box_width * 0.5;
    if right < -200.0 || left > width + 200.0 || bottom < -200.0 || top > height + 200.0 {
        return None;
    }
    let rect = [left, top, box_width, box_height];
    rect.iter().all(|v| v.is_finite()).then_some(rect)
}
