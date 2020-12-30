use super::AnyPositionConstraint;
use crate::dynamics::{IntegrationParameters, RigidBodySet};
use crate::geometry::{ContactManifold, KinematicsCategory};
use crate::math::{
    AngularInertia, Isometry, Point, Rotation, Translation, Vector, MAX_MANIFOLD_POINTS,
};
use crate::utils::{WAngularInertia, WCross, WDot};

pub(crate) struct PositionGroundConstraint {
    pub rb2: usize,
    // NOTE: the points are relative to the center of masses.
    pub p1: [Point<f32>; MAX_MANIFOLD_POINTS],
    pub local_p2: [Point<f32>; MAX_MANIFOLD_POINTS],
    pub dists: [f32; MAX_MANIFOLD_POINTS],
    pub n1: Vector<f32>,
    pub num_contacts: u8,
    pub im2: f32,
    pub ii2: AngularInertia<f32>,
    pub erp: f32,
    pub max_linear_correction: f32,
}

impl PositionGroundConstraint {
    pub fn generate(
        params: &IntegrationParameters,
        manifold: &ContactManifold,
        bodies: &RigidBodySet,
        out_constraints: &mut Vec<AnyPositionConstraint>,
        push: bool,
    ) {
        let mut rb1 = &bodies[manifold.data.body_pair.body1];
        let mut rb2 = &bodies[manifold.data.body_pair.body2];
        let flip = !rb2.is_dynamic();

        let n1 = if flip {
            std::mem::swap(&mut rb1, &mut rb2);
            -manifold.data.normal
        } else {
            manifold.data.normal
        };

        let active_contacts = &manifold.data.solver_contacts[..manifold.num_active_contacts];

        for (l, manifold_contacts) in active_contacts.chunks(MAX_MANIFOLD_POINTS).enumerate() {
            let mut p1 = [Point::origin(); MAX_MANIFOLD_POINTS];
            let mut local_p2 = [Point::origin(); MAX_MANIFOLD_POINTS];
            let mut dists = [0.0; MAX_MANIFOLD_POINTS];

            for k in 0..manifold_contacts.len() {
                p1[k] = manifold_contacts[k].point;
                local_p2[k] = rb2
                    .position
                    .inverse_transform_point(&manifold_contacts[k].point);
                dists[k] = manifold_contacts[k].dist;
            }

            let constraint = PositionGroundConstraint {
                rb2: rb2.active_set_offset,
                p1,
                local_p2,
                n1,
                dists,
                im2: rb2.mass_properties.inv_mass,
                ii2: rb2.world_inv_inertia_sqrt.squared(),
                num_contacts: manifold_contacts.len() as u8,
                erp: params.erp,
                max_linear_correction: params.max_linear_correction,
            };

            if push {
                if manifold.kinematics.category == KinematicsCategory::PointPoint {
                    out_constraints.push(AnyPositionConstraint::NongroupedPointPointGround(
                        constraint,
                    ));
                } else {
                    out_constraints.push(AnyPositionConstraint::NongroupedPlanePointGround(
                        constraint,
                    ));
                }
            } else {
                if manifold.kinematics.category == KinematicsCategory::PointPoint {
                    out_constraints[manifold.data.constraint_index + l] =
                        AnyPositionConstraint::NongroupedPointPointGround(constraint);
                } else {
                    out_constraints[manifold.data.constraint_index + l] =
                        AnyPositionConstraint::NongroupedPlanePointGround(constraint);
                }
            }
        }
    }
    pub fn solve_point_point(
        &self,
        params: &IntegrationParameters,
        positions: &mut [Isometry<f32>],
    ) {
        // FIXME: can we avoid most of the multiplications by pos1/pos2?
        // Compute jacobians.
        let mut pos2 = positions[self.rb2];
        let allowed_err = params.allowed_linear_error;

        for k in 0..self.num_contacts as usize {
            let target_dist = -self.dists[k] - allowed_err;
            let p1 = self.p1[k];
            let p2 = pos2 * self.local_p2[k];
            let dpos = p2 - p1;

            let sqdist = dpos.norm_squared();

            // NOTE: only works for the point-point case.
            if sqdist < target_dist * target_dist {
                let dist = sqdist.sqrt();
                let n = dpos / dist;
                let err = ((dist - target_dist) * self.erp).max(-self.max_linear_correction);
                let dp2 = p2.coords - pos2.translation.vector;

                let gcross2 = -dp2.gcross(n);
                let ii_gcross2 = self.ii2.transform_vector(gcross2);

                // Compute impulse.
                let inv_r = self.im2 + gcross2.gdot(ii_gcross2);
                let impulse = err / inv_r;

                // Apply impulse.
                let tra2 = Translation::from(n * (-impulse * self.im2));
                let rot2 = Rotation::new(ii_gcross2 * impulse);
                pos2 = Isometry::from_parts(tra2 * pos2.translation, rot2 * pos2.rotation);
            }
        }

        positions[self.rb2] = pos2;
    }

    pub fn solve_plane_point(
        &self,
        params: &IntegrationParameters,
        positions: &mut [Isometry<f32>],
    ) {
        // FIXME: can we avoid most of the multiplications by pos1/pos2?
        // Compute jacobians.
        let mut pos2 = positions[self.rb2];
        let allowed_err = params.allowed_linear_error;

        for k in 0..self.num_contacts as usize {
            let target_dist = -self.dists[k] - allowed_err;
            let n1 = self.n1;
            let p1 = self.p1[k];
            let p2 = pos2 * self.local_p2[k];
            let dpos = p2 - p1;
            let dist = dpos.dot(&n1);

            if dist < target_dist {
                let err = ((dist - target_dist) * self.erp).max(-self.max_linear_correction);
                let dp2 = p2.coords - pos2.translation.vector;

                let gcross2 = -dp2.gcross(n1);
                let ii_gcross2 = self.ii2.transform_vector(gcross2);

                // Compute impulse.
                let inv_r = self.im2 + gcross2.gdot(ii_gcross2);
                let impulse = err / inv_r;

                // Apply impulse.
                let tra2 = Translation::from(n1 * (-impulse * self.im2));
                let rot2 = Rotation::new(ii_gcross2 * impulse);
                pos2 = Isometry::from_parts(tra2 * pos2.translation, rot2 * pos2.rotation);
            }
        }

        positions[self.rb2] = pos2;
    }
}
