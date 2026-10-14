//! Procedural hair: the head's braid chains trail behind the head instead of staying rigid.
//!
//! The head mesh is skinned to its own 47-bone skeleton, so posing its `Bn_Hair*` chain bones moves
//! the braids. Each braid is simulated as a small rope — Verlet integration with distance
//! constraints — pinned at its socket, so gravity and the head's motion make it swing and settle.
//! The bone rotations are then recovered by swinging each bone's modelled direction onto the
//! simulated segment, which is exactly the pose the skinning consumes.
//!
//! This is a presentation-only effect: nothing here feeds the simulation.

use bevy::prelude::*;

use crate::content::Skeleton;

/// Downward acceleration on the braids, m/s².
const GRAVITY: f32 = -4.0;
/// Velocity retained per frame (air drag); a little under 1 settles the swing.
const DAMPING: f32 = 0.9;
/// How strongly a braid is drawn back toward its modelled (bind) shape, per second. Without some
/// memory a long braid would just hang straight down as a limp rope.
const STIFFNESS: f32 = 16.0;
/// Distance-constraint iterations per frame.
const ITERATIONS: usize = 6;

/// The head's skeleton bones and the braid chains the dynamics drive. Lives on the head attach node.
#[derive(Component)]
pub struct Hair {
    /// Head bone entities, indexed like the head skeleton.
    bones: Vec<Entity>,
    /// Each head bone's bind transform in the attach node's frame.
    bind_world: Vec<Mat4>,
    chains: Vec<Chain>,
}

/// One braid: an ordered run of bones from its socket (pinned) to its tip.
struct Chain {
    /// Head skeleton bone indices, base to tip.
    bones: Vec<usize>,
    /// Rest distance between consecutive bone origins, metres.
    lengths: Vec<f32>,
    /// Simulated bone-origin positions, world space, and the previous frame's.
    pos: Vec<Vec3>,
    prev: Vec<Vec3>,
    /// Whether `pos`/`prev` have been seeded from the current head pose.
    seeded: bool,
}

impl Hair {
    /// Build the chains from the head skeleton. Braid roots are the `Bn_Hair*_Parent` bones; each
    /// chain follows its single-child run out to the tip.
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

        let mut chains = Vec::new();
        for root in 0..n {
            let name = &skeleton.names[root];
            if !name.starts_with("Bn_Hair") || !name.ends_with("_Parent") {
                continue;
            }
            let mut chain_bones = vec![root];
            loop {
                let parent = *chain_bones.last().unwrap() as i32;
                let children: Vec<usize> =
                    (0..n).filter(|&k| skeleton.parents[k] == parent).collect();
                if children.len() == 1 {
                    chain_bones.push(children[0]);
                } else {
                    break;
                }
            }
            if chain_bones.len() < 2 {
                continue;
            }
            let lengths = chain_bones
                .windows(2)
                .map(|w| {
                    bind_world[w[0]]
                        .transform_point3(Vec3::ZERO)
                        .distance(bind_world[w[1]].transform_point3(Vec3::ZERO))
                })
                .collect();
            let count = chain_bones.len();
            chains.push(Chain {
                bones: chain_bones,
                lengths,
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

/// Advance the braids. Runs after the locomotion pose, so the head has already moved this frame.
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

            // Integrate (Verlet): the pinned socket follows the head; the rest keep their momentum,
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
                *pos = p + (rest[j] - p) * (STIFFNESS * dt).min(1.0);
            }

            // Distance constraints (follow-the-leader from the pinned socket).
            for _ in 0..ITERATIONS {
                chain.pos[0] = rest[0];
                for j in 1..chain.pos.len() {
                    let delta = chain.pos[j] - chain.pos[j - 1];
                    let length = delta.length().max(1e-6);
                    let target = chain.pos[j - 1] + delta / length * chain.lengths[j - 1];
                    chain.pos[j] = target;
                }
            }

            // Recover bone rotations: swing each bone's modelled direction onto the simulated
            // segment, then re-express the result as a local rotation.
            let mut parent_world_rot = node_rot * bind_rotation(bind_world, chain.bones[0]);
            for j in 1..chain.bones.len() {
                let bone = chain.bones[j];
                let bind_dir = (rest[j] - rest[j - 1]).normalize_or_zero();
                let dir = (chain.pos[j] - chain.pos[j - 1]).normalize_or_zero();
                let swing = if bind_dir.length_squared() > 0.0 && dir.length_squared() > 0.0 {
                    Quat::from_rotation_arc(bind_dir, dir)
                } else {
                    Quat::IDENTITY
                };
                let world_rot = swing * (node_rot * bind_rotation(bind_world, bone));
                if let Ok(mut transform) = transforms.get_mut(bones[bone]) {
                    transform.rotation = parent_world_rot.inverse() * world_rot;
                }
                parent_world_rot = world_rot;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Skeleton;

    /// A three-bone chain (socket + two segments) in a straight line down Bevy -Y, so the bind
    /// direction is trivial to check against.
    fn skeleton() -> Skeleton {
        let down = |y: f32| Transform::from_translation(Vec3::new(0.0, y, 0.0));
        Skeleton {
            names: vec![
                "Bn_Socket_Hair".into(),
                "Bn_Hair30_Parent".into(),
                "Bn_Hair30_1".into(),
                "Bn_Hair30_11".into(),
            ],
            parents: vec![-1, 0, 1, 2],
            locals: vec![down(0.0), down(0.0), down(-0.25), down(-0.25)],
            inverse_bindposes: vec![Mat4::IDENTITY; 4],
            bind: Vec::new(),
        }
    }

    /// The chain the dynamics find runs from the socket to the tip, with the segment lengths taken
    /// from the bind pose.
    #[test]
    fn braid_chains_follow_the_bone_run() {
        let hair = Hair::new(&skeleton(), vec![Entity::PLACEHOLDER; 4]);
        assert_eq!(hair.chains.len(), 1, "one braid");
        let chain = &hair.chains[0];
        assert_eq!(chain.bones, vec![1, 2, 3], "socket to tip, in order");
        assert_eq!(chain.lengths, vec![0.25, 0.25]);
    }
}
