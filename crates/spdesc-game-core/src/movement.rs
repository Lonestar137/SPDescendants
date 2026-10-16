//! The character movement rule: a capsule walking, jumping, and turning on ground at a given
//! height.
//!
//! This is the project-owned simulation of The First Descendant's `M1CharacterMovementComponent`
//! at the fidelity budget the plan records: recorded tuning keeps its game units, discrete rules
//! are exact, and timing is expressed per simulation step. The game's own movement tuning lives in
//! the character Blueprint's component defaults under `TFD_DATA`; until that is extracted
//! ([`MovementParams::default`] documents the chosen local rule), the numbers here are a
//! deliberate, single-player stand-in.
//!
//! Coordinates are the game's: TFD (Unreal) is left-handed with **+X forward, +Y right, +Z up**
//! and **centimetres**. Yaw is degrees counter-clockwise from +X; the character's facing yaw
//! follows its movement direction. Movement is resolved against a *movement frame* yaw supplied by
//! the caller (world-relative movement passes `0`; a camera-relative scheme passes the camera's
//! yaw). The simulation never reads a clock: [`Character::step`] takes the elapsed seconds.

/// Tuning for [`Character::step`]. `Default` is the documented local rule, not extracted data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementParams {
    /// Ground speed while walking, cm/s.
    pub walk_speed: f64,
    /// Ground speed while sprinting, cm/s.
    pub sprint_speed: f64,
    /// Upward velocity imparted by a jump, cm/s.
    pub jump_z_velocity: f64,
    /// Downward acceleration, cm/s² (positive number).
    pub gravity: f64,
    /// Horizontal acceleration while airborne, cm/s².
    pub air_accel: f64,
    /// Turn rate toward the movement direction, degrees per second.
    pub turn_rate: f64,
    /// Turn rate toward the aim direction while aiming, degrees per second. Aiming turns the
    /// character onto its aim rather than its movement, and does so briskly.
    pub aim_turn_rate: f64,
    /// How far below the feet the ground may fall while the character is still *walking* on it,
    /// centimetres. Ground is not a plane: a sloped surface, a kerb walked down, a step in the
    /// world's triangles all put the floor a little below where it was, and a walker that treated
    /// each as a fall would spend the whole walk in the air — and the whole walk playing landings.
    /// Anything further below than this is a ledge, and a walker falls off it.
    pub step_down: f64,
}

impl Default for MovementParams {
    /// Chosen local rule (recorded 2026-10-13, `QUESTIONS.md`): there is no extracted movement
    /// tuning yet, so these are typical Unreal character values in the game's units. Walking is a
    /// deliberate, readable pace; sprint doubles it; the jump arcs about 1.8 m.
    fn default() -> Self {
        Self {
            walk_speed: 400.0,
            sprint_speed: 800.0,
            jump_z_velocity: 600.0,
            gravity: 980.0,
            air_accel: 800.0,
            turn_rate: 720.0,
            aim_turn_rate: 1440.0,
            step_down: 45.0,
        }
    }
}

/// The player's movement intent for one step, already resolved from input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveIntent {
    /// Forward input, `-1..=1` (positive is forward, away from the camera).
    pub forward: f64,
    /// Right input, `-1..=1` (positive is right).
    pub right: f64,
    /// Whether sprint is held.
    pub sprint: bool,
    /// Whether jump was pressed this step.
    pub jump: bool,
    /// While aiming, the yaw to face regardless of movement (strafe), degrees. `None` follows the
    /// movement direction instead.
    pub aim_yaw: Option<f64>,
}

/// A character's kinematic state. Lengths are centimetres, angles degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Character {
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    /// Facing yaw, degrees counter-clockwise from +X. Follows the movement direction.
    pub yaw: f64,
    pub grounded: bool,
}

impl Default for Character {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
            yaw: 0.0,
            grounded: true,
        }
    }
}

impl Character {
    /// Unit facing vector in the horizontal plane.
    pub fn facing(&self) -> [f64; 3] {
        let y = self.yaw.to_radians();
        [y.cos(), y.sin(), 0.0]
    }

    /// Advance one step of `dt` seconds. `frame_yaw` is the yaw the input is relative to (the
    /// camera's), `ground_height` the floor's Z (a flat plane is the MVP map). Movement is
    /// horizontal-instant on the ground and accelerated in the air; the facing yaw turns toward
    /// the movement direction; gravity always applies.
    pub fn step(
        &mut self,
        params: &MovementParams,
        mv: MoveIntent,
        frame_yaw: f64,
        ground_height: f64,
        dt: f64,
    ) {
        // Jump, before integrating so the impulse is visible this step.
        if self.grounded && mv.jump {
            self.velocity[2] = params.jump_z_velocity;
            self.grounded = false;
        }

        // Desired direction from the intent, in the movement frame.
        let (cy, sy) = {
            let r = frame_yaw.to_radians();
            (r.cos(), r.sin())
        };
        let forward = [cy, sy];
        // The game frame is left-handed (+X forward, +Y right, +Z up), so a forward-facing
        // character's right is the *clockwise* perpendicular of forward, `(sin, -cos)`. (Taking the
        // counter-clockwise one puts the player's right on their left.)
        let right = [sy, -cy];
        let mut wish = [
            forward[0] * mv.forward + right[0] * mv.right,
            forward[1] * mv.forward + right[1] * mv.right,
        ];
        let len = wish[0].hypot(wish[1]);
        if len > 1e-9 {
            wish[0] /= len;
            wish[1] /= len;
        } else {
            wish = [0.0, 0.0];
        }
        // Facing: aiming pins it to the aim yaw (so the character strafes while it shoots),
        // otherwise it turns toward the movement direction. Either way the turn respects a rate.
        let (target, rate) = if let Some(aim) = mv.aim_yaw {
            (aim, params.aim_turn_rate)
        } else if len > 1e-9 {
            (
                wish[1].atan2(wish[0]).to_degrees().rem_euclid(360.0),
                params.turn_rate,
            )
        } else {
            (self.yaw, params.turn_rate)
        };
        let delta = (target - self.yaw + 540.0).rem_euclid(360.0) - 180.0;
        let max_turn = rate * dt;
        self.yaw = (self.yaw + delta.clamp(-max_turn, max_turn)).rem_euclid(360.0);
        let speed = if mv.sprint {
            params.sprint_speed
        } else {
            params.walk_speed
        };
        let desired = [wish[0] * speed, wish[1] * speed];

        if self.grounded {
            // Ground: reach the desired velocity at once (chosen local rule).
            self.velocity[0] = desired[0];
            self.velocity[1] = desired[1];
        } else {
            // Air: steer toward the desired velocity at the air-control rate.
            let max_delta = params.air_accel * dt;
            for (v, target) in self.velocity.iter_mut().zip(desired) {
                *v += (target - *v).clamp(-max_delta, max_delta);
            }
        }

        // Gravity and integration.
        self.velocity[2] -= params.gravity * dt;
        for axis in 0..3 {
            self.position[axis] += self.velocity[axis] * dt;
        }

        // Floor. A walker already on the ground keeps it across a drop no deeper than a step, so
        // grounded ground does not turn into a fall — and a landing — every few centimetres. The
        // snap only applies to a walker that was *on* the ground: one that has just jumped has
        // left it, and one further above is falling.
        if self.position[2] <= ground_height + 1e-9 {
            self.position[2] = ground_height;
            if self.velocity[2] < 0.0 {
                self.velocity[2] = 0.0;
            }
            self.grounded = true;
        } else if self.grounded && self.position[2] - ground_height <= params.step_down {
            self.position[2] = ground_height;
            self.velocity[2] = 0.0;
            self.grounded = true;
        } else {
            self.grounded = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk() -> (MovementParams, Character) {
        (MovementParams::default(), Character::default())
    }

    #[test]
    fn walking_moves_along_the_frame_direction() {
        let (p, mut c) = walk();
        c.step(
            &p,
            MoveIntent {
                forward: 1.0,
                ..MoveIntent::default()
            },
            0.0,
            0.0,
            0.1,
        );
        // Frame yaw 0 faces +X: one step at 400 cm/s for 0.1 s is 40 cm, and nothing on Y.
        assert!((c.position[0] - 40.0).abs() < 1e-6);
        assert!(c.position[1].abs() < 1e-9);
        assert_eq!(c.position[2], 0.0);
        assert_eq!(c.yaw, 0.0, "the character faces the direction it moves");
    }

    #[test]
    fn right_input_is_the_player_right_in_the_left_handed_frame() {
        let (p, mut c) = walk();
        c.step(
            &p,
            MoveIntent {
                right: 1.0,
                ..MoveIntent::default()
            },
            0.0,
            0.0,
            0.1,
        );
        // The game frame is left-handed, so a character facing +X (yaw 0) has its right along -Y.
        assert!(c.position[0].abs() < 1e-6, "no forward component");
        assert!((c.position[1] + 40.0).abs() < 1e-6, "right moves toward -Y");
    }

    #[test]
    fn movement_is_relative_to_the_frame_yaw() {
        let (p, mut c) = walk();
        // Camera faces +Y; forward input moves the character to +Y and it turns to face +Y.
        c.step(
            &p,
            MoveIntent {
                forward: 1.0,
                ..MoveIntent::default()
            },
            90.0,
            0.0,
            0.1,
        );
        assert!(c.position[0].abs() < 1e-6);
        assert!((c.position[1] - 40.0).abs() < 1e-6);
        // The turn toward the movement direction is rate-limited, but it starts turning.
        assert!(c.yaw > 0.0 && c.yaw <= 90.0);
    }

    #[test]
    fn facing_turns_toward_the_movement_direction_within_the_turn_rate() {
        let (p, mut c) = walk();
        // Facing +X, asked to move toward +Y (frame 90°, forward): the turn is clamped by the
        // turn rate over a very short step.
        c.step(
            &p,
            MoveIntent {
                forward: 1.0,
                ..MoveIntent::default()
            },
            90.0,
            0.0,
            0.001,
        );
        assert!(c.yaw > 0.0 && c.yaw < 90.0, "yaw {0}", c.yaw);
    }

    #[test]
    fn jump_leaves_the_ground_then_gravity_returns_it() {
        let (p, mut c) = walk();
        let mut peak = 0.0f64;
        let mut airborne_steps = 0;
        for i in 0..400 {
            let jump = i == 0;
            c.step(
                &p,
                MoveIntent {
                    jump,
                    ..MoveIntent::default()
                },
                0.0,
                0.0,
                0.005,
            );
            peak = peak.max(c.position[2]);
            if !c.grounded {
                airborne_steps += 1;
            }
        }
        assert!(peak > 150.0, "the jump must clear 1.5 m, got {peak}");
        assert!(
            airborne_steps > 200,
            "the character must be airborne a while"
        );
        assert_eq!(c.position[2], 0.0, "and land back on the floor");
        assert!(c.grounded);
    }

    /// Aiming pins the facing to the aim yaw while the input still strafes, so the character faces
    /// where it shoots rather than where it moves.
    #[test]
    fn aiming_pins_the_facing_to_the_aim_yaw_while_strafing() {
        let p = MovementParams::default();
        let mut c = Character::default();
        c.step(
            &p,
            MoveIntent {
                right: 1.0,
                aim_yaw: Some(90.0),
                ..MoveIntent::default()
            },
            0.0,
            0.0,
            0.1,
        );
        assert!(
            (c.yaw - 90.0).abs() < 1e-9,
            "aiming faces the aim yaw, got {}",
            c.yaw
        );
        assert!(
            c.position[0].hypot(c.position[1]) > 0.0,
            "the character still strafes while aiming"
        );
    }

    /// Ground that steps *down* within a step is still walked on: the walker stays grounded and
    /// does not fall, which is what keeps uneven ground from turning into a fall every step.
    #[test]
    fn a_step_down_keeps_the_walker_on_the_ground() {
        let p = MovementParams::default();
        let mut c = Character::default();
        // Walking forward over ground that falls away 20 cm a step, for a second.
        let mut ground = 0.0;
        let mut airborne = 0;
        for i in 0..200 {
            if i > 0 && i % 20 == 0 {
                ground -= 20.0;
            }
            c.step(&p, MoveIntent::default(), 0.0, ground, 0.005);
            if !c.grounded {
                airborne += 1;
            }
        }
        assert_eq!(airborne, 0, "the walker left the ground {airborne} times");
        assert_eq!(c.position[2], ground);

        // A real ledge is still a fall: 2 m below is not a step.
        let mut c = Character::default();
        c.step(&p, MoveIntent::default(), 0.0, -200.0, 0.005);
        assert!(!c.grounded, "a two-metre drop is a fall");
    }

    #[test]
    fn sprint_is_faster_than_walk() {
        let p = MovementParams::default();
        let mut a = Character::default();
        let mut b = Character::default();
        let mv = |sprint| MoveIntent {
            forward: 1.0,
            sprint,
            ..MoveIntent::default()
        };
        a.step(&p, mv(false), 0.0, 0.0, 0.1);
        b.step(&p, mv(true), 0.0, 0.0, 0.1);
        assert!(b.position[0] > a.position[0]);
    }

    #[test]
    fn diagonal_input_is_normalised() {
        let (p, mut c) = walk();
        c.step(
            &p,
            MoveIntent {
                forward: 1.0,
                right: 1.0,
                ..MoveIntent::default()
            },
            0.0,
            0.0,
            0.1,
        );
        let dist = (c.position[0].powi(2) + c.position[1].powi(2)).sqrt();
        assert!((dist - 40.0).abs() < 1e-6, "diagonal speed equals cardinal");
    }
}
