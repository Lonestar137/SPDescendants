//! Procedural hair: the head's hair chains trail behind the head instead of staying rigid.
//!
//! The head mesh is skinned to its own skeleton, so posing its `Bn_Hair*` bones moves the hair. A
//! root bone's whole subtree is simulated as a small rope — Verlet integration with parent-to-child
//! distance constraints — pinned at its socket, so gravity and the head's motion make it swing and
//! settle. Bone rotations are recovered by swinging each bone's modelled direction onto the
//! simulated one, which is exactly the pose the skinning consumes. Branches are simulated too (a
//! bone's other children just ride its rotation), so twin-tails and layered cuts do not tear.
//!
//! This is a presentation-only effect: nothing here feeds the simulation.

use bevy::prelude::*;

use crate::content::Skeleton;

/// Downward acceleration on the hair, m/s².
const GRAVITY: f32 = -4.0;
/// Velocity retained per frame (air drag); a little under 1 settles the swing.
const DAMPING: f32 = 0.9;
/// How strongly a chain is drawn back toward its modelled (bind) shape, per second. Without some
/// memory a long chain would just hang straight down as a limp rope.
const STIFFNESS: f32 = 16.0;
/// Distance-constraint iterations per frame.
const ITERATIONS: usize = 6;
/// How far (metres) a simulated joint may leave its modelled position; a guard against a bad bind
/// pose or a frame hitch throwing the hair across the map.
const MAX_OFFSET: f32 = 0.35;
/// The most a bone may swing from its modelled direction, radians.
const MAX_SWING: f32 = 1.2;

/// The head's skeleton bones and the hair chains the dynamics drive. Lives on the head attach node.
#[derive(Component)]
pub struct Hair {
    /// Head bone entities, indexed like the head skeleton.
    bones: Vec<Entity>,
    /// Each head bone's bind transform in the attach node's frame.
    bind_world: Vec<Mat4>,
    chains: Vec<Chain>,
}

/// One simulated subtree: its bones in parent-before-child order, with the links and bind data the
/// dynamics need.
struct Chain {
    /// Head skeleton bone indices, parent before child.
    bones: Vec<usize>,
    /// For each entry, the index (within `bones`) of its chain parent.
    parent: Vec<Option<usize>>,
    /// Rest distance from the parent's origin, metres.
    length: Vec<f32>,
    /// For each entry, the index (within `bones`) of the primary child to aim at, when it has one.
    aim: Vec<Option<usize>>,
    /// Each bone's bind world rotation in the attach node's frame.
    bind_rot: Vec<Quat>,
    /// Simulated bone-origin positions, world space, and the previous frame's.
    pos: Vec<Vec3>,
    prev: Vec<Vec3>,
    /// Whether `pos`/`prev` have been seeded from the current head pose.
    seeded: bool,
}

impl Hair {
    /// Build the chains from the head skeleton. Each `Bn_Hair*_Parent` roots one chain containing
    /// its whole subtree.
    pub fn new(skeleton: &Skeleton, bones: Vec<Entity>) -> Self {
        let n = skeleton.names.len();
        let mut bind_world = vec![Mat4::IDENTITY; n];
        for i in 0..n {
            let local = skeleton.locals[i];
            let m = Mat4::from_scale_rotation_translation(
                local.scale,
                local.rotation,
                local.translation,
            );
            bind_world[i] = if skeleton.parents[i] >= 0 {
                bind_world[skeleton.parents[i] as usize] * m
            } else {
                m
            };
        }
        let origin = |b: usize| bind_world[b].transform_point3(Vec3::ZERO);

        let mut chains = Vec::new();
        for root in 0..n {
            let name = &skeleton.names[root];
            if !name.starts_with("Bn_Hair") || !name.ends_with("_Parent") {
                continue;
            }
            // The whole subtree, parent before child.
            let mut chain_bones = vec![root];
            let mut cursor = 0;
            while cursor < chain_bones.len() {
                let parent = chain_bones[cursor] as i32;
                for k in 0..n {
                    if skeleton.parents[k] == parent {
                        chain_bones.push(k);
                    }
                }
                cursor += 1;
            }
            if chain_bones.len() < 2 {
                continue;
            }

            let mut parent = Vec::with_capacity(chain_bones.len());
            let mut length = Vec::with_capacity(chain_bones.len());
            let mut aim = Vec::with_capacity(chain_bones.len());
            let mut bind_rot = Vec::with_capacity(chain_bones.len());
            for (j, &bone) in chain_bones.iter().enumerate() {
                let p = (skeleton.parents[bone] >= 0)
                    .then(|| {
                        chain_bones[..j]
                            .iter()
                            .position(|&x| x == skeleton.parents[bone] as usize)
                    })
                    .flatten();
                parent.push(p);
                length.push(p.map_or(0.0, |p| origin(chain_bones[p]).distance(origin(bone))));
                aim.push(
                    (0..n)
                        .find(|&k| skeleton.parents[k] == bone as i32)
                        .and_then(|child| chain_bones.iter().position(|&x| x == child)),
                );
                bind_rot.push(bind_rotation(&bind_world, bone));
            }
            let count = chain_bones.len();
            chains.push(Chain {
                bones: chain_bones,
                parent,
                length,
                aim,
                bind_rot,
                pos: vec![Vec3::ZERO; count],
                prev: vec![Vec3::ZERO; count],
                seeded: false,
            });
        }

        Hair {
            bones,
            bind_world,
            chains,
        }
    }
}

/// The rotation part of a bind transform.
fn bind_rotation(bind_world: &[Mat4], bone: usize) -> Quat {
    bind_world[bone].to_scale_rotation_translation().1
}

/// `from_rotation_arc`, with the angle clamped so a degenerate bind cannot spin a bone wildly.
fn swing(from: Vec3, to: Vec3) -> Quat {
    if from.length_squared() <= 0.0 || to.length_squared() <= 0.0 {
        return Quat::IDENTITY;
    }
    let rotation = Quat::from_rotation_arc(from, to);
    let (axis, angle) = rotation.to_axis_angle();
    if angle <= MAX_SWING || !axis.is_finite() {
        rotation
    } else {
        Quat::from_axis_angle(axis, MAX_SWING)
    }
}

/// Advance the hair. Runs after the locomotion pose, so the head has already moved this frame.
pub fn animate_hair(
    time: Res<Time>,
    mut heads: Query<(&mut Hair, &GlobalTransform)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs().clamp(0.0, 1.0 / 20.0);
    if dt <= 0.0 {
        return;
    }
    for (mut hair, node) in &mut heads {
        let node_mat = node.to_matrix();
        let node_rot = node.rotation();
        let Hair {
            bones,
            bind_world,
            chains,
        } = &mut *hair;
        for chain in chains.iter_mut() {
            // Where each bone origin sits at bind, in world space, right now.
            let rest: Vec<Vec3> = chain
                .bones
                .iter()
                .map(|&b| node_mat.transform_point3(bind_world[b].transform_point3(Vec3::ZERO)))
                .collect();

            if !chain.seeded {
                chain.pos.clone_from(&rest);
                chain.prev.clone_from(&rest);
                chain.seeded = true;
            }

            // Integrate (Verlet): the pinned root follows the head; the rest keep their momentum,
            // fall a little, and are drawn back toward their modelled shape.
            chain.pos[0] = rest[0];
            chain.prev[0] = rest[0];
            for (j, (pos, prev)) in chain
                .pos
                .iter_mut()
                .zip(chain.prev.iter_mut())
                .enumerate()
                .skip(1)
            {
                let velocity = (*pos - *prev) * DAMPING;
                *prev = *pos;
                let mut p = *pos + velocity;
                p.y += GRAVITY * dt * dt;
                p += (rest[j] - p) * (STIFFNESS * dt).min(1.0);
                *pos = rest[j] + (p - rest[j]).clamp_length_max(MAX_OFFSET);
            }

            // Distance constraints, parent to child.
            for _ in 0..ITERATIONS {
                chain.pos[0] = rest[0];
                for j in 1..chain.pos.len() {
                    let Some(p) = chain.parent[j] else {
                        continue;
                    };
                    let delta = chain.pos[j] - chain.pos[p];
                    let length = delta.length().max(1e-6);
                    chain.pos[j] = chain.pos[p] + delta / length * chain.length[j];
                }
            }

            // Recover bone rotations: swing each bone's modelled aim onto the simulated one, then
            // re-express the result as a local rotation. A bone with no child here keeps its bind.
            let mut world_rot: Vec<Quat> = chain.bind_rot.iter().map(|r| node_rot * *r).collect();
            for j in 0..chain.bones.len() {
                let Some(p) = chain.parent[j] else {
                    continue;
                };
                if let Some(c) = chain.aim[j] {
                    let bind_dir = (rest[c] - rest[j]).normalize_or_zero();
                    let dir = (chain.pos[c] - chain.pos[j]).normalize_or_zero();
                    world_rot[j] = swing(bind_dir, dir) * (node_rot * chain.bind_rot[j]);
                }
                if let Ok(mut transform) = transforms.get_mut(bones[chain.bones[j]]) {
                    transform.rotation = world_rot[p].inverse() * world_rot[j];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Skeleton;

    fn skeleton() -> Skeleton {
        let down = |y: f32| Transform::from_translation(Vec3::new(0.0, y, 0.0));
        Skeleton {
            names: vec![
                "Bn_Socket_Hair".into(),
                "Bn_Hair30_Parent".into(),
                "Bn_Hair30_1".into(),
                "Bn_Hair30_11".into(),
                "Bn_Hair30_12".into(),
            ],
            parents: vec![-1, 0, 1, 2, 2],
            locals: vec![down(0.0), down(0.0), down(-0.25), down(-0.25), down(-0.25)],
            inverse_bindposes: vec![Mat4::IDENTITY; 5],
            bind: Vec::new(),
        }
    }

    /// A root's whole subtree becomes one chain, including a branch (the tip has two children).
    #[test]
    fn braid_chains_cover_the_whole_subtree() {
        let hair = Hair::new(&skeleton(), vec![Entity::PLACEHOLDER; 5]);
        assert_eq!(hair.chains.len(), 1, "one hair root");
        let chain = &hair.chains[0];
        assert_eq!(chain.bones, vec![1, 2, 3, 4], "root then both tips");
        assert_eq!(chain.parent, vec![None, Some(0), Some(1), Some(1)]);
        assert_eq!(chain.length, vec![0.0, 0.25, 0.25, 0.25]);
        // The parent aims at the first of its children; the tip aims at the first tip.
        assert_eq!(chain.aim, vec![Some(1), Some(2), None, None]);
    }

    /// The swing helper never exceeds the clamp.
    #[test]
    fn swing_is_clamped() {
        let opposite = swing(Vec3::Y, Vec3::NEG_Y);
        assert!(opposite.to_axis_angle().1 <= MAX_SWING + 1e-6);
    }
}
