//! The projectile/hitscan weapon rule: firing, hit detection, and damage.
//!
//! The First Descendant's weapons are hit-scan with a fire interval, a magazine, and a per-shot
//! damage that feeds the stat/damage pipeline. This is the project-owned simulation of the shooting
//! loop at the fidelity budget the plan records: the *rules* (cooldown, mag/reload, nearest-hit,
//! health) are exact and deterministic, while the *numbers* are a documented local rule until the
//! weapon's own data-table row and damage formula are extracted (see `MECHANICS-MAP.md`). The
//! simulation never reads a clock or an RNG: [`Combat::try_fire`] takes the elapsed time, and a
//! spread cone is resolved from a caller-supplied direction.
//!
//! Coordinates are the game's: left-handed, `+X` forward, `+Y` right, `+Z` up, centimetres.

/// Weapon tuning. `Default` is a documented local rule, not extracted data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponParams {
    /// Damage per shot.
    pub damage: f64,
    /// Minimum seconds between shots.
    pub fire_interval: f64,
    /// Maximum hit distance, cm.
    pub range: f64,
    /// Rounds per magazine.
    pub magazine: u32,
    /// Seconds to reload a full magazine.
    pub reload_time: f64,
}

impl Default for WeaponParams {
    /// Chosen local rule (recorded 2026-10-14): an automatic rifle. The game's exact values live in
    /// the weapon's `M1RangedWeaponData` row and the damage pipeline; these are a readable stand-in.
    fn default() -> Self {
        Self {
            damage: 120.0,
            fire_interval: 0.1,
            range: 60_000.0,
            magazine: 30,
            reload_time: 2.0,
        }
    }
}

/// A damageable target: a sphere the hit-scan can strike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub center: [f64; 3],
    pub radius: f64,
    pub health: f64,
}

impl Target {
    pub fn new(center: [f64; 3], radius: f64, health: f64) -> Self {
        Self {
            center,
            radius,
            health,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.health > 0.0
    }
}

/// The shot caller's state: cooldown and ammunition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Combat {
    /// Seconds still to wait before the next shot may fire.
    pub cooldown: f64,
    /// Rounds left in the magazine.
    pub ammo: u32,
    /// Seconds left of an in-progress reload, or `0` when not reloading.
    pub reload_remaining: f64,
}

impl Default for Combat {
    fn default() -> Self {
        Self {
            cooldown: 0.0,
            ammo: WeaponParams::default().magazine,
            reload_remaining: 0.0,
        }
    }
}

/// Why a trigger pull did not produce a shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireError {
    /// The weapon is still cooling down between shots.
    Cooling,
    /// The magazine is empty; reload first.
    Empty,
    /// A reload is in progress.
    Reloading,
}

/// What a shot did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shot {
    /// Nothing was in the ray's path within range; `point` is the ray's far end.
    Miss { point: [f64; 3] },
    /// The ray struck target `index` (into the caller's slice) at `point`.
    Hit {
        index: usize,
        point: [f64; 3],
        damage: f64,
        killed: bool,
    },
}

impl Combat {
    /// Advance timers by `dt` seconds (cooldown and reload), completing a reload when it elapses.
    pub fn tick(&mut self, params: &WeaponParams, dt: f64) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        if self.reload_remaining > 0.0 {
            self.reload_remaining -= dt;
            if self.reload_remaining <= 0.0 {
                self.reload_remaining = 0.0;
                self.ammo = params.magazine;
            }
        }
    }

    /// Begin a reload if the magazine is not already full and no reload is in progress.
    pub fn start_reload(&mut self, params: &WeaponParams) {
        if self.reload_remaining == 0.0 && self.ammo < params.magazine {
            self.reload_remaining = params.reload_time;
        }
    }

    /// Fire one shot along `direction` (a unit vector) from `origin`. On the first target the ray
    /// strikes within range, applies `damage` (clamped to its remaining health) and reports it.
    pub fn try_fire(
        &mut self,
        params: &WeaponParams,
        origin: [f64; 3],
        direction: [f64; 3],
        targets: &mut [Target],
    ) -> Result<Shot, FireError> {
        if self.reload_remaining > 0.0 {
            return Err(FireError::Reloading);
        }
        if self.ammo == 0 {
            return Err(FireError::Empty);
        }
        if self.cooldown > 0.0 {
            return Err(FireError::Cooling);
        }
        self.ammo -= 1;
        self.cooldown = params.fire_interval;

        let dir = normalize(direction);
        let mut best: Option<(f64, usize, [f64; 3])> = None;
        for (index, target) in targets.iter().enumerate() {
            if !target.is_alive() {
                continue;
            }
            if let Some(t) = ray_sphere(origin, dir, target.center, target.radius)
                && t <= params.range
                && best.is_none_or(|(bt, _, _)| t < bt)
            {
                best = Some((t, index, add(origin, scale(dir, t))));
            }
        }
        match best {
            Some((_, index, point)) => {
                let target = &mut targets[index];
                let damage = params.damage.min(target.health);
                target.health -= damage;
                Ok(Shot::Hit {
                    index,
                    point,
                    damage,
                    killed: !target.is_alive(),
                })
            }
            None => Ok(Shot::Miss {
                point: add(origin, scale(dir, params.range)),
            }),
        }
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if len <= 0.0 {
        [1.0, 0.0, 0.0]
    } else {
        scale(a, 1.0 / len)
    }
}

/// The near hit distance of a ray against a sphere, or `None`. `dir` must be unit length.
fn ray_sphere(origin: [f64; 3], dir: [f64; 3], center: [f64; 3], radius: f64) -> Option<f64> {
    let m = sub(origin, center);
    let b = dot(m, dir);
    let c = dot(m, m) - radius * radius;
    if c > 0.0 && b > 0.0 {
        return None; // origin outside and ray pointing away
    }
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    Some(if t < 0.0 { 0.0 } else { t })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight() -> [f64; 3] {
        [1.0, 0.0, 0.0]
    }

    #[test]
    fn a_shot_hits_the_first_target_in_line() {
        let p = WeaponParams::default();
        let mut c = Combat::default();
        let mut targets = vec![
            Target::new([100.0, 0.0, 0.0], 20.0, 1000.0),
            Target::new([200.0, 0.0, 0.0], 20.0, 1000.0),
        ];
        let shot = c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap();
        match shot {
            Shot::Hit {
                index,
                point,
                damage,
                killed,
            } => {
                assert_eq!(index, 0);
                assert_eq!(point, [80.0, 0.0, 0.0]);
                assert_eq!(damage, p.damage);
                assert!(!killed);
            }
            other => panic!("expected a hit, got {other:?}"),
        }
        assert_eq!(c.ammo, p.magazine - 1);
    }

    #[test]
    fn a_miss_reports_the_far_end_of_the_ray() {
        let p = WeaponParams::default();
        let mut c = Combat::default();
        let mut targets = vec![Target::new([100.0, 500.0, 0.0], 20.0, 1000.0)];
        let shot = c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap();
        assert!(matches!(shot, Shot::Miss { .. }));
    }

    #[test]
    fn a_distant_target_is_out_of_range() {
        let p = WeaponParams {
            range: 150.0,
            ..Default::default()
        };
        let mut c = Combat::default();
        let mut targets = vec![Target::new([1000.0, 0.0, 0.0], 20.0, 1000.0)];
        assert!(matches!(
            c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap(),
            Shot::Miss { .. }
        ));
    }

    #[test]
    fn fire_interval_gates_the_trigger() {
        let p = WeaponParams::default();
        let mut c = Combat::default();
        let mut targets = vec![Target::new([100.0, 0.0, 0.0], 20.0, 1e9)];
        c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap();
        assert_eq!(
            c.try_fire(&p, [0.0; 3], straight(), &mut targets),
            Err(FireError::Cooling)
        );
        c.tick(&p, p.fire_interval);
        assert!(c.try_fire(&p, [0.0; 3], straight(), &mut targets).is_ok());
    }

    #[test]
    fn the_magazine_empties_and_reloads() {
        let p = WeaponParams {
            magazine: 3,
            ..Default::default()
        };
        let mut c = Combat {
            ammo: 3,
            ..Default::default()
        };
        c.cooldown = 0.0;
        for _ in 0..3 {
            c.cooldown = 0.0;
            assert!(c.try_fire(&p, [0.0; 3], straight(), &mut []).is_ok());
        }
        assert_eq!(
            c.try_fire(&p, [0.0; 3], straight(), &mut []),
            Err(FireError::Empty)
        );
        c.start_reload(&p);
        assert_eq!(
            c.try_fire(&p, [0.0; 3], straight(), &mut []),
            Err(FireError::Reloading)
        );
        c.tick(&p, p.reload_time);
        assert_eq!(c.ammo, 3, "the reload fills the magazine");
        c.cooldown = 0.0;
        assert!(c.try_fire(&p, [0.0; 3], straight(), &mut []).is_ok());
    }

    #[test]
    fn damage_kills_and_a_dead_target_is_skipped() {
        let p = WeaponParams {
            damage: 60.0,
            ..Default::default()
        };
        let mut c = Combat::default();
        let mut targets = vec![Target::new([100.0, 0.0, 0.0], 20.0, 100.0)];
        for _ in 0..2 {
            c.cooldown = 0.0;
            c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap();
        }
        assert!(!targets[0].is_alive());
        // A third shot passes through the corpse.
        c.cooldown = 0.0;
        assert!(matches!(
            c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap(),
            Shot::Miss { .. }
        ));
    }

    #[test]
    fn the_nearest_of_two_overlapping_targets_is_hit() {
        let p = WeaponParams::default();
        let mut c = Combat::default();
        let mut targets = vec![
            Target::new([300.0, 0.0, 0.0], 40.0, 1000.0),
            Target::new([100.0, 0.0, 0.0], 40.0, 1000.0),
        ];
        let shot = c.try_fire(&p, [0.0; 3], straight(), &mut targets).unwrap();
        assert!(matches!(shot, Shot::Hit { index: 1, .. }));
    }
}
