use super::{position::BlockPos, vector3::Vector3};

#[derive(Clone, Copy)]
pub struct BoundingBox {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

impl BoundingBox {
    pub fn new_default(size: &BoundingBoxSize) -> Self {
        Self::new_from_pos(0., 0., 0., size)
    }

    pub fn new_from_pos(x: f64, y: f64, z: f64, size: &BoundingBoxSize) -> Self {
        let f = size.width / 2.;
        Self {
            min_x: x - f,
            min_y: y,
            min_z: z - f,
            max_x: x + f,
            max_y: y + size.height,
            max_z: z + f,
        }
    }

    pub fn new(min: Vector3<f64>, max: Vector3<f64>) -> Self {
        Self {
            min_x: min.x,
            min_y: min.y,
            min_z: min.z,
            max_x: max.x,
            max_y: max.y,
            max_z: max.z,
        }
    }

    pub fn from_block(position: &BlockPos) -> Self {
        let position = position.0;
        Self {
            min_x: position.x as f64,
            min_y: position.y as f64,
            min_z: position.z as f64,
            max_x: (position.x as f64) + 1.0,
            max_y: (position.y as f64) + 1.0,
            max_z: (position.z as f64) + 1.0,
        }
    }

    pub fn intersects(&self, other: &BoundingBox) -> bool {
        self.min_x < other.max_x
            && self.max_x > other.min_x
            && self.min_y < other.max_y
            && self.max_y > other.min_y
            && self.min_z < other.max_z
            && self.max_z > other.min_z
    }

    pub fn intersects_block(&self, position: &BlockPos, bounding_box: &[f32]) -> bool {
        for i in 0..bounding_box.len() / 6 {
            let other = BoundingBox {
                min_x: position.0.x as f64 + bounding_box[i * 6] as f64,
                min_y: position.0.y as f64 + bounding_box[i * 6 + 1] as f64,
                min_z: position.0.z as f64 + bounding_box[i * 6 + 2] as f64,
                max_x: position.0.x as f64 + bounding_box[i * 6 + 3] as f64,
                max_y: position.0.y as f64 + bounding_box[i * 6 + 4] as f64,
                max_z: position.0.z as f64 + bounding_box[i * 6 + 5] as f64,
            };
            if self.intersects(&other) {
                return true;
            }
        }
        false
    }

    pub fn squared_magnitude(&self, pos: Vector3<f64>) -> f64 {
        let d = f64::max(f64::max(self.min_x - pos.x, pos.x - self.max_x), 0.0);
        let e = f64::max(f64::max(self.min_y - pos.y, pos.y - self.max_y), 0.0);
        let f = f64::max(f64::max(self.min_z - pos.z, pos.z - self.max_z), 0.0);
        super::squared_magnitude(d, e, f)
    }
}

#[derive(Clone, Copy)]
pub struct BoundingBoxSize {
    pub width: f64,
    pub height: f64,
}
