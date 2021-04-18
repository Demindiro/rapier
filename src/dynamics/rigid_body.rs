use crate::dynamics::MassProperties;
use crate::geometry::{
    Collider, ColliderHandle, ColliderSet, InteractionGraph, RigidBodyGraphIndex,
};
use crate::math::{
    AngVector, AngularInertia, Isometry, Point, Real, Rotation, Translation, Vector,
};
use crate::utils::{self, WAngularInertia, WCross, WDot};
use na::ComplexField;
use num::Zero;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde-serialize", derive(Serialize, Deserialize))]
/// The status of a body, governing the way it is affected by external forces.
pub enum BodyStatus {
    /// A `BodyStatus::Dynamic` body can be affected by all external forces.
    Dynamic,
    /// A `BodyStatus::Static` body cannot be affected by external forces.
    Static,
    /// A `BodyStatus::Kinematic` body cannot be affected by any external forces but can be controlled
    /// by the user at the position level while keeping realistic one-way interaction with dynamic bodies.
    ///
    /// One-way interaction means that a kinematic body can push a dynamic body, but a kinematic body
    /// cannot be pushed by anything. In other words, the trajectory of a kinematic body can only be
    /// modified by the user and is independent from any contact or joint it is involved in.
    Kinematic,
    // Semikinematic, // A kinematic that performs automatic CCD with the static environment to avoid traversing it?
    // Disabled,
}

#[cfg(feature = "dim3")]
/// Represents a specific direction
pub enum Axis {
    /// X Axis
    X,
    /// Y Axis
    Y,
    /// Z Axis
    Z,
}

bitflags::bitflags! {
    #[cfg_attr(feature = "serde-serialize", derive(Serialize, Deserialize))]
    /// Flags affecting the behavior of the constraints solver for a given contact manifold.
    pub(crate) struct RigidBodyFlags: u8 {
        const TRANSLATION_LOCKED = 1 << 0;
        const ROTATION_LOCKED_X = 1 << 1;
        const ROTATION_LOCKED_Y = 1 << 2;
        const ROTATION_LOCKED_Z = 1 << 3;
        const CCD_ENABLED = 1 << 4;
        const CCD_ACTIVE = 1 << 5;
    }
}

bitflags::bitflags! {
    #[cfg_attr(feature = "serde-serialize", derive(Serialize, Deserialize))]
    /// Flags describing how the rigid-body has been modified by the user.
    pub(crate) struct RigidBodyChanges: u32 {
        const MODIFIED    = 1 << 0;
        const POSITION    = 1 << 1;
        const SLEEP       = 1 << 2;
        const COLLIDERS   = 1 << 3;
        const BODY_STATUS = 1 << 4;
    }
}

#[cfg_attr(feature = "serde-serialize", derive(Serialize, Deserialize))]
/// A rigid body.
///
/// To create a new rigid-body, use the `RigidBodyBuilder` structure.
#[derive(Debug, Clone)]
pub struct RigidBody {
    /// The world-space position of the rigid-body.
    pub(crate) position: Isometry<Real>,
    /// The next position of the rigid-body.
    ///
    /// At the beginning of the timestep, and when the
    /// timestep is complete we must have position == next_position
    /// except for kinematic bodies.
    ///
    /// The next_position is updated after the velocity and position
    /// resolution. Then it is either validated (ie. we set position := set_position)
    /// or clamped by CCD.
    pub(crate) next_position: Isometry<Real>,
    /// The local mass properties of the rigid-body.
    pub(crate) mass_properties: MassProperties,
    /// The world-space center of mass of the rigid-body.
    pub world_com: Point<Real>,
    /// The inverse mass taking into account translation locking.
    pub effective_inv_mass: Real,
    /// The square-root of the world-space inverse angular inertia tensor of the rigid-body,
    /// taking into account rotation locking.
    pub effective_world_inv_inertia_sqrt: AngularInertia<Real>,
    /// The linear velocity of the rigid-body.
    pub(crate) linvel: Vector<Real>,
    /// The angular velocity of the rigid-body.
    pub(crate) angvel: AngVector<Real>,
    /// Damping factor for gradually slowing down the translational motion of the rigid-body.
    pub linear_damping: Real,
    /// Damping factor for gradually slowing down the angular motion of the rigid-body.
    pub angular_damping: Real,
    /// Accumulation of external forces (only for dynamic bodies).
    pub(crate) force: Vector<Real>,
    /// Accumulation of external torques (only for dynamic bodies).
    pub(crate) torque: AngVector<Real>,
    pub(crate) colliders: Vec<ColliderHandle>,
    pub(crate) gravity_scale: Real,
    /// Whether or not this rigid-body is sleeping.
    pub activation: ActivationStatus,
    pub(crate) joint_graph_index: RigidBodyGraphIndex,
    pub(crate) active_island_id: usize,
    pub(crate) active_set_id: usize,
    pub(crate) active_set_offset: usize,
    pub(crate) active_set_timestamp: u32,
    flags: RigidBodyFlags,
    pub(crate) changes: RigidBodyChanges,
    /// The status of the body, governing how it is affected by external forces.
    body_status: BodyStatus,
    /// The dominance group this rigid-body is part of.
    dominance_group: i8,
    /// User-defined data associated to this rigid-body.
    pub user_data: u128,
    pub(crate) ccd_thickness: Real,
    pub(crate) ccd_max_dist: Real,
}

impl RigidBody {
    fn new() -> Self {
        Self {
            position: Isometry::identity(),
            next_position: Isometry::identity(),
            mass_properties: MassProperties::zero(),
            world_com: Point::origin(),
            effective_inv_mass: 0.0,
            effective_world_inv_inertia_sqrt: AngularInertia::zero(),
            linvel: Vector::zeros(),
            angvel: na::zero(),
            force: Vector::zeros(),
            torque: na::zero(),
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            colliders: Vec::new(),
            activation: ActivationStatus::new_active(),
            joint_graph_index: InteractionGraph::<(), ()>::invalid_graph_index(),
            active_island_id: 0,
            active_set_id: 0,
            active_set_offset: 0,
            active_set_timestamp: 0,
            flags: RigidBodyFlags::empty(),
            changes: RigidBodyChanges::all(),
            body_status: BodyStatus::Dynamic,
            dominance_group: 0,
            user_data: 0,
            ccd_thickness: Real::MAX,
            ccd_max_dist: 0.0,
        }
    }

    pub(crate) fn reset_internal_references(&mut self) {
        self.colliders = Vec::new();
        self.joint_graph_index = InteractionGraph::<(), ()>::invalid_graph_index();
        self.active_island_id = 0;
        self.active_set_id = 0;
        self.active_set_offset = 0;
        self.active_set_timestamp = 0;
    }

    pub(crate) fn add_gravity(&mut self, gravity: Vector<Real>) {
        if self.effective_inv_mass != 0.0 {
            self.force += gravity * self.gravity_scale * self.mass();
        }
    }

    #[cfg(not(feature = "parallel"))] // in parallel solver this is not needed
    pub(crate) fn integrate_accelerations(&mut self, dt: Real) {
        let linear_acc = self.force * self.effective_inv_mass;
        let angular_acc = self.effective_world_inv_inertia_sqrt
            * (self.effective_world_inv_inertia_sqrt * self.torque);

        self.linvel += linear_acc * dt;
        self.angvel += angular_acc * dt;
    }

    /// The status of this rigid-body.
    pub fn body_status(&self) -> BodyStatus {
        self.body_status
    }

    /// Sets the status of this rigid-body.
    pub fn set_body_status(&mut self, status: BodyStatus) {
        if status != self.body_status {
            self.changes.insert(RigidBodyChanges::BODY_STATUS);
            self.body_status = status;
        }
    }

    /// The mass properties of this rigid-body.
    #[inline]
    pub fn mass_properties(&self) -> &MassProperties {
        &self.mass_properties
    }

    /// The dominance group of this rigid-body.
    ///
    /// This method always returns `i8::MAX + 1` for non-dynamic
    /// rigid-bodies.
    #[inline]
    pub fn effective_dominance_group(&self) -> i16 {
        if self.is_dynamic() {
            self.dominance_group as i16
        } else {
            i8::MAX as i16 + 1
        }
    }

    /// Are the translations of this rigid-body locked?
    pub fn is_translation_locked(&self) -> bool {
        self.flags.contains(RigidBodyFlags::TRANSLATION_LOCKED)
    }

    /// Sets whether the translation of this rigid-body is locked.
    pub fn set_translation_locked(&mut self, lock: bool) {
        if lock {
            self.flags.insert(RigidBodyFlags::TRANSLATION_LOCKED);
        } else {
            self.flags.remove(RigidBodyFlags::TRANSLATION_LOCKED);
        }
        self.update_world_mass_properties();
    }

    /// Are the rotations of this rigid-body locked?
    #[cfg(feature = "dim2")]
    pub fn is_rotation_locked(&self) -> bool {
        self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_Z)
    }

    /// Returns `true` for each rotational degrees of freedom locked on this rigid-body.
    #[cfg(feature = "dim3")]
    pub fn is_rotation_locked(&self) -> [bool; 3] {
        [
            self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_X),
            self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_Y),
            self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_Z),
        ]
    }

    /// Sets whether the rotation of this rigid-body is locked.
    #[cfg(feature = "dim2")]
    pub fn set_rotation_locked(&mut self, lock: bool) {
        if lock {
            self.flags.insert(RigidBodyFlags::ROTATION_LOCKED_Z);
        } else {
            self.flags.remove(RigidBodyFlags::ROTATION_LOCKED_Z);
        }
        self.update_world_mass_properties();
    }

    /// Sets whether the rotation along a given axis of this rigid-body is locked.
    #[cfg(feature = "dim3")]
    pub fn set_rotation_locked(&mut self, axis: Axis, lock: bool) {
        let flag = match axis {
            Axis::X => RigidBodyFlags::ROTATION_LOCKED_X,
            Axis::Y => RigidBodyFlags::ROTATION_LOCKED_Y,
            Axis::Z => RigidBodyFlags::ROTATION_LOCKED_Z,
        };
        if lock {
            self.flags.insert(flag);
        } else {
            self.flags.remove(flag);
        }
        self.update_world_mass_properties();
    }

    /// Enables of disable CCD (continuous collision-detection) for this rigid-body.
    pub fn enable_ccd(&mut self, enabled: bool) {
        self.flags.set(RigidBodyFlags::CCD_ENABLED, enabled)
    }

    /// Is CCD (continous collision-detection) enabled for this rigid-body?
    pub fn is_ccd_enabled(&self) -> bool {
        self.flags.contains(RigidBodyFlags::CCD_ENABLED)
    }

    // This is different from `is_ccd_enabled`. This checks that CCD
    // is active for this rigid-body, i.e., if it was seen to move fast
    // enough to justify a CCD run.
    /// Is CCD active for this rigid-body?
    ///
    /// The CCD is considered active if the rigid-body is moving at
    /// a velocity greater than an automatically-computed threshold.
    ///
    /// This is not the same as `self.is_ccd_enabled` which only
    /// checks if CCD is allowed to run for this rigid-body or if
    /// it is completely disabled (independently from its velocity).
    pub fn is_ccd_active(&self) -> bool {
        self.flags.contains(RigidBodyFlags::CCD_ACTIVE)
    }

    pub(crate) fn update_ccd_active_flag(&mut self, dt: Real, include_forces: bool) {
        let ccd_active = self.is_ccd_enabled() && self.is_moving_fast(dt, include_forces);
        self.flags.set(RigidBodyFlags::CCD_ACTIVE, ccd_active);
    }

    pub(crate) fn is_moving_fast(&self, dt: Real, include_forces: bool) -> bool {
        if self.is_dynamic() {
            // NOTE: for the threshold we don't use the exact CCD thickness. Theoretically, we
            //       should use `self.ccd_thickness - smallest_contact_dist` where `smallest_contact_dist`
            //       is the deepest contact (the contact with the largest penetration depth, i.e., the
            //       negative `dist` with the largest absolute value.
            //       However, getting this penetration depth assumes querying the contact graph from
            //       the narrow-phase, which can be pretty expensive. So we use the CCD thickness
            //       divided by 10 right now. We will see in practice if this value is OK or if we
            //       should use a smaller (to be less conservative) or larger divisor (to be more conservative).
            let threshold = self.ccd_thickness / 10.0;

            if include_forces {
                let linear_part = (self.linvel + self.force * dt).norm();
                #[cfg(feature = "dim2")]
                let angular_part = (self.angvel + self.torque * dt).abs() * self.ccd_max_dist;
                #[cfg(feature = "dim3")]
                let angular_part = (self.angvel + self.torque * dt).norm() * self.ccd_max_dist;
                let vel_with_forces = linear_part + angular_part;
                vel_with_forces > threshold
            } else {
                self.max_point_velocity() * dt > threshold
            }
        } else {
            false
        }
    }

    pub(crate) fn max_point_velocity(&self) -> Real {
        #[cfg(feature = "dim2")]
        return self.linvel.norm() + self.angvel.abs() * self.ccd_max_dist;
        #[cfg(feature = "dim3")]
        return self.linvel.norm() + self.angvel.norm() * self.ccd_max_dist;
    }

    /// Sets the rigid-body's initial mass properties.
    ///
    /// If `wake_up` is `true` then the rigid-body will be woken up if it was
    /// put to sleep because it did not move for a while.
    #[inline]
    pub fn set_mass_properties(&mut self, props: MassProperties, wake_up: bool) {
        if self.is_dynamic() && wake_up {
            self.wake_up(true);
        }

        self.mass_properties = props;
        self.update_world_mass_properties();
    }

    /// The handles of colliders attached to this rigid body.
    pub fn colliders(&self) -> &[ColliderHandle] {
        &self.colliders[..]
    }

    /// Is this rigid body dynamic?
    ///
    /// A dynamic body can move freely and is affected by forces.
    pub fn is_dynamic(&self) -> bool {
        self.body_status == BodyStatus::Dynamic
    }

    /// Is this rigid body kinematic?
    ///
    /// A kinematic body can move freely but is not affected by forces.
    pub fn is_kinematic(&self) -> bool {
        self.body_status == BodyStatus::Kinematic
    }

    /// Is this rigid body static?
    ///
    /// A static body cannot move and is not affected by forces.
    pub fn is_static(&self) -> bool {
        self.body_status == BodyStatus::Static
    }

    /// The mass of this rigid body.
    ///
    /// Returns zero if this rigid body has an infinite mass.
    pub fn mass(&self) -> Real {
        utils::inv(self.mass_properties.inv_mass)
    }

    /// The predicted position of this rigid-body.
    ///
    /// If this rigid-body is kinematic this value is set by the `set_next_kinematic_position`
    /// method and is used for estimating the kinematic body velocity at the next timestep.
    /// For non-kinematic bodies, this value is currently unspecified.
    pub fn next_position(&self) -> &Isometry<Real> {
        &self.next_position
    }

    /// The scale factor applied to the gravity affecting this rigid-body.
    pub fn gravity_scale(&self) -> Real {
        self.gravity_scale
    }

    /// Sets the gravity scale facter for this rigid-body.
    pub fn set_gravity_scale(&mut self, scale: Real, wake_up: bool) {
        if wake_up && self.activation.sleeping {
            self.changes.insert(RigidBodyChanges::SLEEP);
            self.activation.sleeping = false;
        }

        self.gravity_scale = scale;
    }

    /// Adds a collider to this rigid-body.
    pub(crate) fn add_collider(&mut self, handle: ColliderHandle, coll: &Collider) {
        self.changes.set(
            RigidBodyChanges::MODIFIED | RigidBodyChanges::COLLIDERS,
            true,
        );

        self.ccd_thickness = self.ccd_thickness.min(coll.shape().ccd_thickness());

        let shape_bsphere = coll
            .shape()
            .compute_bounding_sphere(coll.position_wrt_parent());
        self.ccd_max_dist = self
            .ccd_max_dist
            .max(shape_bsphere.center.coords.norm() + shape_bsphere.radius);

        let mass_properties = coll
            .mass_properties()
            .transform_by(coll.position_wrt_parent());
        self.colliders.push(handle);
        self.mass_properties += mass_properties;
        self.update_world_mass_properties();
    }

    pub(crate) fn update_colliders_positions(&mut self, colliders: &mut ColliderSet) {
        for handle in &self.colliders {
            // NOTE: we use `get_mut_internal_with_modification_tracking` here because we want to
            //       benefit from the modification tracking to know the colliders
            //       we need to update the broad-phase and narrow-phase for.
            let collider = colliders
                .get_mut_internal_with_modification_tracking(*handle)
                .unwrap();
            collider.set_position(self.position * collider.delta);
        }
    }

    /// Removes a collider from this rigid-body.
    pub(crate) fn remove_collider_internal(&mut self, handle: ColliderHandle, coll: &Collider) {
        if let Some(i) = self.colliders.iter().position(|e| *e == handle) {
            self.changes.set(RigidBodyChanges::COLLIDERS, true);
            self.colliders.swap_remove(i);
            let mass_properties = coll
                .mass_properties()
                .transform_by(coll.position_wrt_parent());
            self.mass_properties -= mass_properties;
            self.update_world_mass_properties();
        }
    }

    /// Put this rigid body to sleep.
    ///
    /// A sleeping body no longer moves and is no longer simulated by the physics engine unless
    /// it is waken up. It can be woken manually with `self.wake_up` or automatically due to
    /// external forces like contacts.
    pub fn sleep(&mut self) {
        self.activation.energy = 0.0;
        self.activation.sleeping = true;
        self.linvel = na::zero();
        self.angvel = na::zero();
    }

    /// Wakes up this rigid body if it is sleeping.
    ///
    /// If `strong` is `true` then it is assured that the rigid-body will
    /// remain awake during multiple subsequent timesteps.
    pub fn wake_up(&mut self, strong: bool) {
        if self.activation.sleeping {
            self.changes.insert(RigidBodyChanges::SLEEP);
            self.activation.sleeping = false;
        }

        if (strong || self.activation.energy == 0.0) && self.is_dynamic() {
            self.activation.energy = self.activation.threshold.abs() * 2.0;
        }
    }

    pub(crate) fn update_energy(&mut self) {
        let mix_factor = 0.01;
        let new_energy = (1.0 - mix_factor) * self.activation.energy
            + mix_factor * (self.linvel.norm_squared() + self.angvel.gdot(self.angvel));
        self.activation.energy = new_energy.min(self.activation.threshold.abs() * 4.0);
    }

    /// Is this rigid body sleeping?
    pub fn is_sleeping(&self) -> bool {
        // TODO: should we:
        // - return false for static bodies.
        // - return true for non-sleeping dynamic bodies.
        // - return true only for kinematic bodies with non-zero velocity?
        self.activation.sleeping
    }

    /// Is the velocity of this body not zero?
    pub fn is_moving(&self) -> bool {
        !self.linvel.is_zero() || !self.angvel.is_zero()
    }

    /// Computes the predict position of this rigid-body after `dt` seconds, taking
    /// into account its velocities and external forces applied to it.
    pub fn predict_position_using_velocity_and_forces(&self, dt: Real) -> Isometry<Real> {
        let dlinvel = self.force * (self.effective_inv_mass * dt);
        let dangvel = self
            .effective_world_inv_inertia_sqrt
            .transform_vector(self.torque * dt);
        let linvel = self.linvel + dlinvel;
        let angvel = self.angvel + dangvel;

        let com = self.position * self.mass_properties.local_com;
        let shift = Translation::from(com.coords);
        shift * Isometry::new(linvel * dt, angvel * dt) * shift.inverse() * self.position
    }

    pub(crate) fn integrate_velocity(&self, dt: Real) -> Isometry<Real> {
        let com = self.position * self.mass_properties.local_com;
        let shift = Translation::from(com.coords);
        shift * Isometry::new(self.linvel * dt, self.angvel * dt) * shift.inverse()
    }

    pub(crate) fn apply_damping(&mut self, dt: Real) {
        self.linvel *= 1.0 / (1.0 + dt * self.linear_damping);
        self.angvel *= 1.0 / (1.0 + dt * self.angular_damping);
    }

    pub(crate) fn integrate_next_position(&mut self, dt: Real) {
        self.next_position = self.integrate_velocity(dt) * self.position;
        let _ = self.next_position.rotation.renormalize_fast();
    }

    /// The linear velocity of this rigid-body.
    pub fn linvel(&self) -> &Vector<Real> {
        &self.linvel
    }

    /// The angular velocity of this rigid-body.
    #[cfg(feature = "dim2")]
    pub fn angvel(&self) -> Real {
        self.angvel
    }

    /// The angular velocity of this rigid-body.
    #[cfg(feature = "dim3")]
    pub fn angvel(&self) -> &Vector<Real> {
        &self.angvel
    }

    /// The linear velocity of this rigid-body.
    ///
    /// If `wake_up` is `true` then the rigid-body will be woken up if it was
    /// put to sleep because it did not move for a while.
    pub fn set_linvel(&mut self, linvel: Vector<Real>, wake_up: bool) {
        self.linvel = linvel;

        if self.is_dynamic() && wake_up {
            self.wake_up(true)
        }
    }

    /// The angular velocity of this rigid-body.
    ///
    /// If `wake_up` is `true` then the rigid-body will be woken up if it was
    /// put to sleep because it did not move for a while.
    #[cfg(feature = "dim2")]
    pub fn set_angvel(&mut self, angvel: Real, wake_up: bool) {
        self.angvel = angvel;

        if self.is_dynamic() && wake_up {
            self.wake_up(true)
        }
    }

    /// The angular velocity of this rigid-body.
    ///
    /// If `wake_up` is `true` then the rigid-body will be woken up if it was
    /// put to sleep because it did not move for a while.
    #[cfg(feature = "dim3")]
    pub fn set_angvel(&mut self, angvel: Vector<Real>, wake_up: bool) {
        self.angvel = angvel;

        if self.is_dynamic() && wake_up {
            self.wake_up(true)
        }
    }

    /// The world-space position of this rigid-body.
    pub fn position(&self) -> &Isometry<Real> {
        &self.position
    }

    /// Sets the position and `next_kinematic_position` of this rigid body.
    ///
    /// This will teleport the rigid-body to the specified position/orientation,
    /// completely ignoring any physics rule. If this body is kinematic, this will
    /// also set the next kinematic position to the same value, effectively
    /// resetting to zero the next interpolated velocity of the kinematic body.
    ///
    /// If `wake_up` is `true` then the rigid-body will be woken up if it was
    /// put to sleep because it did not move for a while.
    pub fn set_position(&mut self, pos: Isometry<Real>, wake_up: bool) {
        self.changes.insert(RigidBodyChanges::POSITION);
        self.position = pos;
        self.next_position = pos;

        // TODO: Do we really need to check that the body isn't dynamic?
        if wake_up && self.is_dynamic() {
            self.wake_up(true)
        }
    }

    pub(crate) fn set_next_position(&mut self, pos: Isometry<Real>) {
        self.next_position = pos;
    }

    /// If this rigid body is kinematic, sets its future position after the next timestep integration.
    pub fn set_next_kinematic_position(&mut self, pos: Isometry<Real>) {
        if self.is_kinematic() {
            self.next_position = pos;
        }
    }

    pub(crate) fn compute_velocity_from_next_position(&mut self, inv_dt: Real) {
        let dpos = self.next_position * self.position.inverse();
        #[cfg(feature = "dim2")]
        {
            self.angvel = dpos.rotation.angle() * inv_dt;
        }
        #[cfg(feature = "dim3")]
        {
            self.angvel = dpos.rotation.scaled_axis() * inv_dt;
        }
        self.linvel = dpos.translation.vector * inv_dt;
    }

    pub(crate) fn update_world_mass_properties(&mut self) {
        self.world_com = self.mass_properties.world_com(&self.position);
        self.effective_inv_mass = self.mass_properties.inv_mass;
        self.effective_world_inv_inertia_sqrt = self
            .mass_properties
            .world_inv_inertia_sqrt(&self.position.rotation);

        // Take into account translation/rotation locking.
        if self.flags.contains(RigidBodyFlags::TRANSLATION_LOCKED) {
            self.effective_inv_mass = 0.0;
        }

        #[cfg(feature = "dim2")]
        {
            if self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_Z) {
                self.effective_world_inv_inertia_sqrt = 0.0;
            }
        }
        #[cfg(feature = "dim3")]
        {
            if self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_X) {
                self.effective_world_inv_inertia_sqrt.m11 = 0.0;
                self.effective_world_inv_inertia_sqrt.m12 = 0.0;
                self.effective_world_inv_inertia_sqrt.m13 = 0.0;
            }

            if self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_Y) {
                self.effective_world_inv_inertia_sqrt.m22 = 0.0;
                self.effective_world_inv_inertia_sqrt.m12 = 0.0;
                self.effective_world_inv_inertia_sqrt.m23 = 0.0;
            }
            if self.flags.contains(RigidBodyFlags::ROTATION_LOCKED_Z) {
                self.effective_world_inv_inertia_sqrt.m33 = 0.0;
                self.effective_world_inv_inertia_sqrt.m13 = 0.0;
                self.effective_world_inv_inertia_sqrt.m23 = 0.0;
            }
        }
    }
}

/// ## Applying forces and torques
impl RigidBody {
    /// Applies a force at the center-of-mass of this rigid-body.
    /// The force will be applied in the next simulation step.
    /// This does nothing on non-dynamic bodies.
    pub fn apply_force(&mut self, force: Vector<Real>, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.force += force;

            if wake_up {
                self.wake_up(true);
            }
        }
    }

    /// Applies a torque at the center-of-mass of this rigid-body.
    /// The torque will be applied in the next simulation step.
    /// This does nothing on non-dynamic bodies.
    #[cfg(feature = "dim2")]
    pub fn apply_torque(&mut self, torque: Real, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.torque += torque;

            if wake_up {
                self.wake_up(true);
            }
        }
    }

    /// Applies a torque at the center-of-mass of this rigid-body.
    /// The torque will be applied in the next simulation step.
    /// This does nothing on non-dynamic bodies.
    #[cfg(feature = "dim3")]
    pub fn apply_torque(&mut self, torque: Vector<Real>, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.torque += torque;

            if wake_up {
                self.wake_up(true);
            }
        }
    }

    /// Applies a force at the given world-space point of this rigid-body.
    /// The force will be applied in the next simulation step.
    /// This does nothing on non-dynamic bodies.
    pub fn apply_force_at_point(&mut self, force: Vector<Real>, point: Point<Real>, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.force += force;
            self.torque += (point - self.world_com).gcross(force);

            if wake_up {
                self.wake_up(true);
            }
        }
    }
}

/// ## Applying impulses and angular impulses
impl RigidBody {
    /// Applies an impulse at the center-of-mass of this rigid-body.
    /// The impulse is applied right away, changing the linear velocity.
    /// This does nothing on non-dynamic bodies.
    pub fn apply_impulse(&mut self, impulse: Vector<Real>, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.linvel += impulse * self.effective_inv_mass;

            if wake_up {
                self.wake_up(true);
            }
        }
    }

    /// Applies an angular impulse at the center-of-mass of this rigid-body.
    /// The impulse is applied right away, changing the angular velocity.
    /// This does nothing on non-dynamic bodies.
    #[cfg(feature = "dim2")]
    pub fn apply_torque_impulse(&mut self, torque_impulse: Real, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.angvel += self.effective_world_inv_inertia_sqrt
                * (self.effective_world_inv_inertia_sqrt * torque_impulse);

            if wake_up {
                self.wake_up(true);
            }
        }
    }

    /// Applies an angular impulse at the center-of-mass of this rigid-body.
    /// The impulse is applied right away, changing the angular velocity.
    /// This does nothing on non-dynamic bodies.
    #[cfg(feature = "dim3")]
    pub fn apply_torque_impulse(&mut self, torque_impulse: Vector<Real>, wake_up: bool) {
        if self.body_status == BodyStatus::Dynamic {
            self.angvel += self.effective_world_inv_inertia_sqrt
                * (self.effective_world_inv_inertia_sqrt * torque_impulse);

            if wake_up {
                self.wake_up(true);
            }
        }
    }

    /// Applies an impulse at the given world-space point of this rigid-body.
    /// The impulse is applied right away, changing the linear and/or angular velocities.
    /// This does nothing on non-dynamic bodies.
    pub fn apply_impulse_at_point(
        &mut self,
        impulse: Vector<Real>,
        point: Point<Real>,
        wake_up: bool,
    ) {
        let torque_impulse = (point - self.world_com).gcross(impulse);
        self.apply_impulse(impulse, wake_up);
        self.apply_torque_impulse(torque_impulse, wake_up);
    }
}

impl RigidBody {
    /// The velocity of the given world-space point on this rigid-body.
    pub fn velocity_at_point(&self, point: &Point<Real>) -> Vector<Real> {
        let dpt = point - self.world_com;
        self.linvel + self.angvel.gcross(dpt)
    }

    /// The kinetic energy of this body.
    pub fn kinetic_energy(&self) -> Real {
        let mut energy = (self.mass() * self.linvel().norm_squared()) / 2.0;

        #[cfg(feature = "dim2")]
        if !self.effective_world_inv_inertia_sqrt.is_zero() {
            let inertia_sqrt = 1.0 / self.effective_world_inv_inertia_sqrt;
            energy += (inertia_sqrt * self.angvel).powi(2) / 2.0;
        }

        #[cfg(feature = "dim3")]
        if !self.effective_world_inv_inertia_sqrt.is_zero() {
            let inertia_sqrt = self.effective_world_inv_inertia_sqrt.inverse_unchecked();
            energy += (inertia_sqrt * self.angvel).norm_squared() / 2.0;
        }

        energy
    }

    /// The potential energy of this body in a gravity field.
    pub fn gravitational_potential_energy(&self, dt: Real, gravity: Vector<Real>) -> Real {
        let world_com = self.mass_properties().world_com(&self.position).coords;

        // Project position back along velocity vector one half-step (leap-frog)
        // to sync up the potential energy with the kinetic energy:
        let world_com = world_com - self.linvel() * (dt / 2.0);

        -self.mass() * self.gravity_scale() * gravity.dot(&world_com)
    }
}

/// A builder for rigid-bodies.
pub struct RigidBodyBuilder {
    position: Isometry<Real>,
    linvel: Vector<Real>,
    angvel: AngVector<Real>,
    gravity_scale: Real,
    linear_damping: Real,
    angular_damping: Real,
    body_status: BodyStatus,
    flags: RigidBodyFlags,
    mass_properties: MassProperties,
    can_sleep: bool,
    sleeping: bool,
    ccd_enabled: bool,
    dominance_group: i8,
    user_data: u128,
}

impl RigidBodyBuilder {
    /// Initialize a new builder for a rigid body which is either static, dynamic, or kinematic.
    pub fn new(body_status: BodyStatus) -> Self {
        Self {
            position: Isometry::identity(),
            linvel: Vector::zeros(),
            angvel: na::zero(),
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            body_status,
            flags: RigidBodyFlags::empty(),
            mass_properties: MassProperties::zero(),
            can_sleep: true,
            sleeping: false,
            ccd_enabled: false,
            dominance_group: 0,
            user_data: 0,
        }
    }

    /// Initializes the builder of a new static rigid body.
    pub fn new_static() -> Self {
        Self::new(BodyStatus::Static)
    }

    /// Initializes the builder of a new kinematic rigid body.
    pub fn new_kinematic() -> Self {
        Self::new(BodyStatus::Kinematic)
    }

    /// Initializes the builder of a new dynamic rigid body.
    pub fn new_dynamic() -> Self {
        Self::new(BodyStatus::Dynamic)
    }

    /// Sets the scale applied to the gravity force affecting the rigid-body to be created.
    pub fn gravity_scale(mut self, x: Real) -> Self {
        self.gravity_scale = x;
        self
    }

    /// Sets the dominance group of this rigid-body.
    pub fn dominance_group(mut self, group: i8) -> Self {
        self.dominance_group = group;
        self
    }

    /// Sets the initial translation of the rigid-body to be created.
    #[cfg(feature = "dim2")]
    pub fn translation(mut self, x: Real, y: Real) -> Self {
        self.position.translation.x = x;
        self.position.translation.y = y;
        self
    }

    /// Sets the initial translation of the rigid-body to be created.
    #[cfg(feature = "dim3")]
    pub fn translation(mut self, x: Real, y: Real, z: Real) -> Self {
        self.position.translation.x = x;
        self.position.translation.y = y;
        self.position.translation.z = z;
        self
    }

    /// Sets the initial orientation of the rigid-body to be created.
    pub fn rotation(mut self, angle: AngVector<Real>) -> Self {
        self.position.rotation = Rotation::new(angle);
        self
    }

    /// Sets the initial position (translation and orientation) of the rigid-body to be created.
    pub fn position(mut self, pos: Isometry<Real>) -> Self {
        self.position = pos;
        self
    }

    /// An arbitrary user-defined 128-bit integer associated to the rigid-bodies built by this builder.
    pub fn user_data(mut self, data: u128) -> Self {
        self.user_data = data;
        self
    }

    /// Sets the additional mass properties of the rigid-body being built.
    ///
    /// Note that "additional" means that the final mass properties of the rigid-bodies depends
    /// on the initial mass-properties of the rigid-body (set by this method)
    /// to which is added the contributions of all the colliders with non-zero density
    /// attached to this rigid-body.
    ///
    /// Therefore, if you want your provided mass properties to be the final
    /// mass properties of your rigid-body, don't attach colliders to it, or
    /// only attach colliders with densities equal to zero.
    pub fn additional_mass_properties(mut self, props: MassProperties) -> Self {
        self.mass_properties = props;
        self
    }

    /// Prevents this rigid-body from translating because of forces.
    pub fn lock_translations(mut self) -> Self {
        self.flags.set(RigidBodyFlags::TRANSLATION_LOCKED, true);
        self
    }

    /// Prevents this rigid-body from rotating because of forces.
    pub fn lock_rotations(mut self) -> Self {
        self.flags.set(RigidBodyFlags::ROTATION_LOCKED_X, true);
        self.flags.set(RigidBodyFlags::ROTATION_LOCKED_Y, true);
        self.flags.set(RigidBodyFlags::ROTATION_LOCKED_Z, true);
        self
    }

    /// Only allow rotations of this rigid-body around specific coordinate axes.
    #[cfg(feature = "dim3")]
    pub fn restrict_rotations(
        mut self,
        allow_rotations_x: bool,
        allow_rotations_y: bool,
        allow_rotations_z: bool,
    ) -> Self {
        self.flags
            .set(RigidBodyFlags::ROTATION_LOCKED_X, !allow_rotations_x);
        self.flags
            .set(RigidBodyFlags::ROTATION_LOCKED_Y, !allow_rotations_y);
        self.flags
            .set(RigidBodyFlags::ROTATION_LOCKED_Z, !allow_rotations_z);
        self
    }

    /// Sets the additional mass of the rigid-body being built.
    ///
    /// This is only the "additional" mass because the total mass of the  rigid-body is
    /// equal to the sum of this additional mass and the mass computed from the colliders
    /// (with non-zero densities) attached to this rigid-body.
    pub fn additional_mass(mut self, mass: Real) -> Self {
        self.mass_properties.set_mass(mass, false);
        self
    }

    /// Sets the additional mass of the rigid-body being built.
    ///
    /// This is only the "additional" mass because the total mass of the  rigid-body is
    /// equal to the sum of this additional mass and the mass computed from the colliders
    /// (with non-zero densities) attached to this rigid-body.
    #[deprecated(note = "renamed to `additional_mass`.")]
    pub fn mass(self, mass: Real) -> Self {
        self.additional_mass(mass)
    }

    /// Sets the additional angular inertia of this rigid-body.
    ///
    /// This is only the "additional" angular inertia because the total angular inertia of
    /// the rigid-body is equal to the sum of this additional value and the angular inertia
    /// computed from the colliders (with non-zero densities) attached to this rigid-body.
    #[cfg(feature = "dim2")]
    pub fn additional_principal_angular_inertia(mut self, inertia: Real) -> Self {
        self.mass_properties.inv_principal_inertia_sqrt =
            utils::inv(ComplexField::sqrt(inertia.max(0.0)));
        self
    }

    /// Sets the angular inertia of this rigid-body.
    #[cfg(feature = "dim2")]
    #[deprecated(note = "renamed to `additional_principal_angular_inertia`.")]
    pub fn principal_angular_inertia(self, inertia: Real) -> Self {
        self.additional_principal_angular_inertia(inertia)
    }

    /// Use `self.principal_angular_inertia` instead.
    #[cfg(feature = "dim2")]
    #[deprecated(note = "renamed to `additional_principal_angular_inertia`.")]
    pub fn principal_inertia(self, inertia: Real) -> Self {
        self.additional_principal_angular_inertia(inertia)
    }

    /// Sets the additional principal angular inertia of this rigid-body.
    ///
    /// This is only the "additional" angular inertia because the total angular inertia of
    /// the rigid-body is equal to the sum of this additional value and the angular inertia
    /// computed from the colliders (with non-zero densities) attached to this rigid-body.
    #[cfg(feature = "dim3")]
    pub fn additional_principal_angular_inertia(mut self, inertia: AngVector<Real>) -> Self {
        self.mass_properties.inv_principal_inertia_sqrt =
            inertia.map(|e| utils::inv(ComplexField::sqrt(e.max(0.0))));
        self
    }

    /// Sets the principal angular inertia of this rigid-body.
    #[cfg(feature = "dim3")]
    #[deprecated(note = "renamed to `additional_principal_angular_inertia`.")]
    pub fn principal_angular_inertia(self, inertia: AngVector<Real>) -> Self {
        self.additional_principal_angular_inertia(inertia)
    }

    /// Use `self.principal_angular_inertia` instead.
    #[cfg(feature = "dim3")]
    #[deprecated(note = "renamed to `additional_principal_angular_inertia`.")]
    pub fn principal_inertia(self, inertia: AngVector<Real>) -> Self {
        self.additional_principal_angular_inertia(inertia)
    }

    /// Sets the damping factor for the linear part of the rigid-body motion.
    ///
    /// The higher the linear damping factor is, the more quickly the rigid-body
    /// will slow-down its translational movement.
    pub fn linear_damping(mut self, factor: Real) -> Self {
        self.linear_damping = factor;
        self
    }

    /// Sets the damping factor for the angular part of the rigid-body motion.
    ///
    /// The higher the angular damping factor is, the more quickly the rigid-body
    /// will slow-down its rotational movement.
    pub fn angular_damping(mut self, factor: Real) -> Self {
        self.angular_damping = factor;
        self
    }

    /// Sets the initial linear velocity of the rigid-body to be created.
    #[cfg(feature = "dim2")]
    pub fn linvel(mut self, x: Real, y: Real) -> Self {
        self.linvel = Vector::new(x, y);
        self
    }

    /// Sets the initial linear velocity of the rigid-body to be created.
    #[cfg(feature = "dim3")]
    pub fn linvel(mut self, x: Real, y: Real, z: Real) -> Self {
        self.linvel = Vector::new(x, y, z);
        self
    }

    /// Sets the initial angular velocity of the rigid-body to be created.
    pub fn angvel(mut self, angvel: AngVector<Real>) -> Self {
        self.angvel = angvel;
        self
    }

    /// Sets whether or not the rigid-body to be created can sleep if it reaches a dynamic equilibrium.
    pub fn can_sleep(mut self, can_sleep: bool) -> Self {
        self.can_sleep = can_sleep;
        self
    }

    /// Enabled continuous collision-detection for this rigid-body.
    pub fn ccd_enabled(mut self, enabled: bool) -> Self {
        self.ccd_enabled = enabled;
        self
    }

    /// Sets whether or not the rigid-body is to be created asleep.
    pub fn sleeping(mut self, sleeping: bool) -> Self {
        self.sleeping = sleeping;
        self
    }

    /// Build a new rigid-body with the parameters configured with this builder.
    pub fn build(&self) -> RigidBody {
        let mut rb = RigidBody::new();
        rb.next_position = self.position; // FIXME: compute the correct value?
        rb.position = self.position;
        rb.linvel = self.linvel;
        rb.angvel = self.angvel;
        rb.body_status = self.body_status;
        rb.user_data = self.user_data;
        rb.mass_properties = self.mass_properties;
        rb.linear_damping = self.linear_damping;
        rb.angular_damping = self.angular_damping;
        rb.gravity_scale = self.gravity_scale;
        rb.flags = self.flags;
        rb.dominance_group = self.dominance_group;
        rb.enable_ccd(self.ccd_enabled);

        if self.can_sleep && self.sleeping {
            rb.sleep();
        }

        if !self.can_sleep {
            rb.activation.threshold = -1.0;
        }

        rb
    }
}

/// The activation status of a body.
///
/// This controls whether a body is sleeping or not.
/// If the threshold is negative, the body never sleeps.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "serde-serialize", derive(Serialize, Deserialize))]
pub struct ActivationStatus {
    /// The threshold pseudo-kinetic energy bellow which the body can fall asleep.
    pub threshold: Real,
    /// The current pseudo-kinetic energy of the body.
    pub energy: Real,
    /// Is this body already sleeping?
    pub sleeping: bool,
}

impl ActivationStatus {
    /// The default amount of energy bellow which a body can be put to sleep by nphysics.
    pub fn default_threshold() -> Real {
        0.01
    }

    /// Create a new activation status initialised with the default activation threshold and is active.
    pub fn new_active() -> Self {
        ActivationStatus {
            threshold: Self::default_threshold(),
            energy: Self::default_threshold() * 4.0,
            sleeping: false,
        }
    }

    /// Create a new activation status initialised with the default activation threshold and is inactive.
    pub fn new_inactive() -> Self {
        ActivationStatus {
            threshold: Self::default_threshold(),
            energy: 0.0,
            sleeping: true,
        }
    }

    /// Returns `true` if the body is not asleep.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.energy != 0.0
    }
}
