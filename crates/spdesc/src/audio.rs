//! Procedurally synthesised sound effects.
//!
//! The game's audio lives inside Wwise banks rather than as decodable media, so — unlike the meshes
//! and textures — there is nothing in `GAME_DATA` to read. These are generated here instead: small
//! mono 16-bit WAV clips built from seeded noise and sine bursts. That keeps the app self-contained
//! (nothing to ship, nothing to licence) and deterministic (the same seed always gives the same
//! samples), which is also what makes them testable.
//!
//! Every clip is a one-shot: [`play`] spawns a [`AudioPlayer`] that Bevy despawns when it finishes.

use std::f32::consts::TAU;
use std::sync::Arc;

use bevy::audio::{AudioSource, Volume};
use bevy::prelude::*;

/// The sample rate of every generated clip.
const RATE: u32 = 44_100;

/// Every sound the game plays, built once at startup.
#[derive(Resource)]
pub struct Sounds {
    /// The shot itself.
    pub shot: Handle<AudioSource>,
    /// The trigger falling on an empty magazine.
    pub dry: Handle<AudioSource>,
    /// The hitmarker: a target took a non-fatal hit.
    pub hit: Handle<AudioSource>,
    /// A target went down.
    pub kill: Handle<AudioSource>,
    /// A reload begins: the magazine coming out.
    pub reload_start: Handle<AudioSource>,
    /// A reload finishes: the magazine in and the bolt released.
    pub reload_end: Handle<AudioSource>,
    /// A footfall while walking or running.
    pub footstep: Handle<AudioSource>,
    /// Takeoff.
    pub jump: Handle<AudioSource>,
    /// Touchdown.
    pub land: Handle<AudioSource>,
}

impl Sounds {
    /// Build every clip into the audio assets.
    pub fn load(sources: &mut Assets<AudioSource>) -> Sounds {
        let mut add = |samples: Vec<f32>| sources.add(source(&samples));
        Sounds {
            shot: add(shot()),
            dry: add(dry()),
            hit: add(hit()),
            kill: add(kill()),
            reload_start: add(reload_start()),
            reload_end: add(reload_end()),
            footstep: add(footstep()),
            jump: add(jump()),
            land: add(land()),
        }
    }
}

/// Build the sounds once, as a resource every player of them reads.
pub fn setup_sounds(mut commands: Commands, mut sources: ResMut<Assets<AudioSource>>) {
    commands.insert_resource(Sounds::load(&mut sources));
}

/// Play `sound` once, at `volume`, and let it clean itself up.
pub fn play(commands: &mut Commands, sound: &Handle<AudioSource>, volume: f32) {
    commands.spawn((
        AudioPlayer::new(sound.clone()),
        PlaybackSettings {
            volume: Volume::Linear(volume),
            ..PlaybackSettings::DESPAWN
        },
    ));
}

/// A `f32` clip as a mono 16-bit WAV [`AudioSource`].
fn source(samples: &[f32]) -> AudioSource {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = samples.len() as u32 * 2;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        bytes.extend_from_slice(&((clamped * i16::MAX as f32) as i16).to_le_bytes());
    }
    AudioSource {
        bytes: Arc::from(bytes.into_boxed_slice()),
    }
}

/// `seconds` worth of samples, one per sample period.
fn seconds(seconds: f32) -> usize {
    (seconds * RATE as f32).round() as usize
}

/// A tiny deterministic noise source (an LCG), so a clip is reproducible.
struct Noise(u32);

impl Noise {
    fn new(seed: u32) -> Noise {
        Noise(seed | 1)
    }

    /// The next sample, in `-1..=1`.
    fn next(&mut self) -> f32 {
        // Numerical Recipes' LCG constants; the high bits are the well-behaved ones.
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1 << 23) as f32 * 2.0 - 1.0
    }
}

/// A one-pole low pass, for the muffled body of a thump rather than pure white noise.
struct LowPass {
    state: f32,
    alpha: f32,
}

impl LowPass {
    fn new(cutoff_hz: f32) -> LowPass {
        let alpha = (TAU * cutoff_hz / RATE as f32).clamp(0.0, 1.0);
        LowPass { state: 0.0, alpha }
    }

    fn step(&mut self, input: f32) -> f32 {
        self.state += self.alpha * (input - self.state);
        self.state
    }
}

/// The envelope of a percussive hit: an instant attack, exponential decay.
fn decay(t: f32, rate: f32) -> f32 {
    (-t * rate).exp()
}

/// A gunshot: a bright noise crack over a short low thump, with a little tail.
fn shot() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0001);
    let mut body = LowPass::new(220.0);
    let mut hiss = LowPass::new(6_000.0);
    (0..seconds(0.22))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let crack = hiss.step(noise.next()) * decay(t, 90.0);
            let thump = (TAU * 95.0 * t).sin() * decay(t, 26.0) * 0.7;
            let grain = body.step(noise.next()) * decay(t, 30.0) * 0.5;
            (crack * 0.9 + thump + grain) * 0.8
        })
        .collect()
}

/// The dry trigger: one dull click and nothing else.
fn dry() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0002);
    let mut click = LowPass::new(2_500.0);
    (0..seconds(0.07))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            click.step(noise.next()) * decay(t, 140.0) * 0.6
        })
        .collect()
}

/// The hitmarker: two short, bright pings a fifth apart, so a hit is unmistakable.
fn hit() -> Vec<f32> {
    (0..seconds(0.12))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let high = (TAU * 1_760.0 * t).sin() * decay(t, 55.0);
            let low = (TAU * 1_180.0 * t).sin() * decay(t, 45.0) * 0.6;
            (high + low) * 0.5
        })
        .collect()
}

/// A target going down: the hitmarker's pings dropped and drawn out, plus a soft thud.
fn kill() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0003);
    let mut body = LowPass::new(400.0);
    (0..seconds(0.35))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            // A falling tone: the frequency slides down as the envelope fades.
            let sweep = (TAU * (900.0 - 520.0 * t / 0.35) * t).sin() * decay(t, 11.0);
            let thud = body.step(noise.next()) * decay(t, 16.0) * 0.6;
            (sweep * 0.5 + thud) * 0.8
        })
        .collect()
}

/// A magazine coming out: two mechanical clicks.
fn reload_start() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0004);
    let mut click = LowPass::new(3_500.0);
    (0..seconds(0.26))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let first = click.step(noise.next()) * decay(t, 120.0);
            let second_at = (t - 0.12).max(0.0);
            let second = (TAU * 620.0 * second_at).sin()
                * decay(second_at, 40.0)
                * if t >= 0.12 { 1.0 } else { 0.0 }
                * 0.5;
            first * 0.55 + second
        })
        .collect()
}

/// A magazine going in and the bolt released: a clack and a short ring.
fn reload_end() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0005);
    let mut click = LowPass::new(4_000.0);
    let mut body = LowPass::new(900.0);
    (0..seconds(0.3))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let clack = click.step(noise.next()) * decay(t, 55.0);
            let ring = (TAU * 1_480.0 * t).sin() * decay(t, 26.0) * 0.35;
            let settle = body.step(noise.next()) * decay(t, 40.0) * 0.4;
            (clack * 0.7 + ring + settle) * 0.8
        })
        .collect()
}

/// A footfall: a muffled noise thump, short enough to fire twice a stride.
fn footstep() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0006);
    let mut body = LowPass::new(700.0);
    (0..seconds(0.13))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let thump = body.step(noise.next()) * decay(t, 45.0);
            let low = (TAU * 120.0 * t).sin() * decay(t, 40.0) * 0.5;
            (thump + low) * 0.7
        })
        .collect()
}

/// Takeoff: an airy sweep up as the legs leave the ground.
fn jump() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0007);
    let mut band = LowPass::new(1_600.0);
    (0..seconds(0.24))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            // Rises to a peak a third of the way in, then falls away.
            let envelope = (t / 0.08).min(1.0) * decay((t - 0.08).max(0.0), 16.0);
            let air = band.step(noise.next()) * envelope;
            let lift = (TAU * (420.0 + 600.0 * t / 0.24) * t).sin() * envelope * 0.3;
            (air * 0.6 + lift) * 0.5
        })
        .collect()
}

/// Touchdown: a heavier, longer thump than a footfall.
fn land() -> Vec<f32> {
    let mut noise = Noise::new(0x5eed_0008);
    let mut body = LowPass::new(320.0);
    (0..seconds(0.3))
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let thump = body.step(noise.next()) * decay(t, 26.0);
            let low = (TAU * 78.0 * t).sin() * decay(t, 20.0) * 0.8;
            (thump + low) * 0.9
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every clip is a well-formed mono 16-bit WAV that a decoder can find the samples in, and is
    /// loud enough to be audible without clipping away.
    #[test]
    fn clips_are_well_formed_and_audible() {
        for (name, samples) in [
            ("shot", shot()),
            ("dry", dry()),
            ("hit", hit()),
            ("kill", kill()),
            ("reload_start", reload_start()),
            ("reload_end", reload_end()),
            ("footstep", footstep()),
            ("jump", jump()),
            ("land", land()),
        ] {
            assert!(!samples.is_empty(), "{name} has no samples");
            assert!(
                samples.iter().any(|s| s.abs() > 0.05),
                "{name} is effectively silent"
            );
            let source = source(&samples);
            let bytes: &[u8] = &source.bytes;
            assert_eq!(&bytes[0..4], b"RIFF", "{name} is not a RIFF file");
            assert_eq!(&bytes[8..12], b"WAVE", "{name} is not WAVE");
            assert_eq!(bytes.len(), 44 + samples.len() * 2, "{name} header size");
            let declared = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
            assert_eq!(declared, samples.len() * 2, "{name} data chunk length");
            let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
            assert_eq!(riff, 36 + declared, "{name} RIFF length");
        }
    }

    /// The generator is seeded, so a clip is identical every run.
    #[test]
    fn clips_are_deterministic() {
        assert_eq!(shot(), shot());
        assert_eq!(footstep(), footstep());
    }
}
