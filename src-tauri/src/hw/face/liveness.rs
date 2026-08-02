//! Liveness math: Eye Aspect Ratio (EAR) and head-pose challenge logic.
//!
//! This module contains the *pure math* of the active-liveness challenge —
//! the same approach as the reference (`face_hello/liveness.py`): measure
//! blink via EAR from eye landmark points, and head turns via yaw from a
//! solvePnP-style pose estimate, and require a random challenge (blink N
//! times / turn left / turn right) before recognizing.
//!
//! The landmark data itself comes from MediaPipe FaceLandmarker (468 points,
//! see [`super::models`]). This module only needs the 3D positions of the
//! 6 eye points + a pose estimate, so it is fully unit-testable without
//! camera or models.

/// Landmark indices used by MediaPipe FaceMesh for the eyes.
pub const LEFT_EYE: [usize; 6] = [33, 160, 158, 133, 153, 144];
pub const RIGHT_EYE: [usize; 6] = [362, 385, 387, 263, 373, 380];

/// Eye Aspect Ratio: (vertical distances) / (2 × horizontal distance).
///
/// `p1..p6` are the 6 eye landmark (x, y) pairs:
/// p1=(33)  p2=(160)  p3=(158)  p4=(133)  p5=(153)  p6=(144) for the left eye
/// (and mirrored indices for the right eye).
/// EAR ≈ 0.25–0.35 open; < ~0.20 closed.
pub fn eye_aspect_ratio(eye: &[(f32, f32)]) -> f32 {
    if eye.len() != 6 {
        return 0.0;
    }
    let d = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let v1 = d(eye[1], eye[5]); // 160–144
    let v2 = d(eye[2], eye[4]); // 158–153
    let h = d(eye[0], eye[3]); // 33–133
    if h <= 1e-6 {
        return 0.0;
    }
    (v1 + v2) / (2.0 * h)
}

/// Average EAR over both eyes.
pub fn average_ear(left: &[(f32, f32)], right: &[(f32, f32)]) -> f32 {
    (eye_aspect_ratio(left) + eye_aspect_ratio(right)) / 2.0
}

/// Challenge kinds for the random liveness prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Challenge {
    Blink(u32),
    TurnLeft,
    TurnRight,
}

/// A discrete liveness state machine.
///
/// Feed it per-frame measurements (average EAR + yaw) and it tracks whether
/// the required challenge has been satisfied. Mirrors the reference
/// `LivenessSession` behavior at the level needed for correct auth gating.
#[derive(Debug, Clone)]
pub struct LivenessState {
    pub challenge: Challenge,
    /// Blink tracking: how many completed blinks so far (for Blink challenge).
    blinks_done: u32,
    /// Whether the previous frame had the eyes closed.
    was_closed: bool,
    /// Blink threshold (EAR below = closed).
    blink_threshold: f32,
    /// Yaw threshold for turn challenges (degrees-ish, from pose).
    yaw_threshold: f32,
    /// Frames since a turn was satisfied (anti-cheat decay).
    turn_satisfied_frames: u32,
    /// Completed.
    pub done: bool,
    /// Passed (challenge satisfied); only meaningful when done.
    pub passed: bool,
}

impl LivenessState {
    pub fn new(challenge: Challenge) -> Self {
        Self {
            challenge,
            blinks_done: 0,
            was_closed: false,
            blink_threshold: 0.21,
            yaw_threshold: 0.20,
            turn_satisfied_frames: 0,
            done: false,
            passed: false,
        }
    }

    /// Random challenge factory (blink 1–2 times, or a turn).
    pub fn random_challenge() -> Challenge {
        // Deterministic-ish for tests; the caller seeds with real entropy.
        let n = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            % 4) as u32;
        match n {
            0 => Challenge::Blink(1),
            1 => Challenge::Blink(2),
            2 => Challenge::TurnLeft,
            _ => Challenge::TurnRight,
        }
    }

    /// Feed one frame of measurements.
    /// - `avg_ear` — average eye aspect ratio (0 = fully closed).
    /// - `yaw` — head yaw (-1..1, 0 = straight; positive = left).
    ///
    /// Returns a human-readable instruction for the lock-screen tile.
    pub fn update(&mut self, avg_ear: f32, yaw: f32) -> &'static str {
        match self.challenge {
            Challenge::Blink(n) => {
                let closed = avg_ear < self.blink_threshold;
                if !closed && self.was_closed {
                    // Eyes re-opened after being closed → one completed blink.
                    self.blinks_done += 1;
                    if self.blinks_done >= n {
                        self.done = true;
                        self.passed = true;
                        return "liveness_ok";
                    }
                }
                self.was_closed = closed;
                "blink"
            }
            Challenge::TurnLeft | Challenge::TurnRight => {
                let target = match self.challenge {
                    Challenge::TurnLeft => self.yaw_threshold,
                    _ => -self.yaw_threshold,
                };
                let satisfied = if self.challenge == Challenge::TurnLeft {
                    yaw > target
                } else {
                    yaw < target
                };
                if satisfied {
                    self.turn_satisfied_frames += 1;
                    if self.turn_satisfied_frames >= 3 {
                        self.done = true;
                        self.passed = true;
                        return "liveness_ok";
                    }
                } else {
                    self.turn_satisfied_frames = 0;
                }
                if self.challenge == Challenge::TurnLeft {
                    "turn_left"
                } else {
                    "turn_right"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 6-point eye polygon with a controllable openness factor.
    /// `open` 0..=1 scales the vertical distance between the corners.
    /// With h=1.0 and v=open*0.25: EAR = (0.25*open + 0.25*open)/(2*1) = 0.25*open.
    fn eye(open: f32) -> [(f32, f32); 6] {
        // MediaPipe eye indices: 0=(33),1=(160),2=(158),3=(133),4=(153),5=(144)
        // Horizontal line at y=0; vertical pairs pushed ±open*0.25.
        [
            (0.0, 0.0),
            (0.25, -open * 0.25),
            (0.5, -open * 0.25),
            (1.0, 0.0),
            (0.5, open * 0.25),
            (0.25, open * 0.25),
        ]
    }

    #[test]
    fn ear_open_vs_closed() {
        let open = eye(1.0);
        let closed = eye(0.05);
        let ear_open = eye_aspect_ratio(&open);
        let ear_closed = eye_aspect_ratio(&closed);
        assert!(ear_open > 0.2, "open EAR {ear_open} should be > 0.2");
        assert!(
            ear_closed < 0.15,
            "closed EAR {ear_closed} should be < 0.15"
        );
        assert!(ear_open > ear_closed * 3.0);
    }

    #[test]
    fn ear_wrong_length_returns_zero() {
        assert_eq!(eye_aspect_ratio(&[(0.0, 0.0); 3]), 0.0);
        assert_eq!(eye_aspect_ratio(&[]), 0.0);
    }

    #[test]
    fn blink_challenge_counts_blinks() {
        let mut s = LivenessState::new(Challenge::Blink(2));
        assert!(!s.done);
        // Open frames
        assert_eq!(s.update(0.30, 0.0), "blink");
        // First blink: close then open
        assert_eq!(s.update(0.10, 0.0), "blink");
        assert_eq!(s.update(0.30, 0.0), "blink");
        // Second blink: close then open → completes
        assert_eq!(s.update(0.10, 0.0), "blink");
        let instr = s.update(0.30, 0.0);
        assert_eq!(instr, "liveness_ok");
        assert!(s.done && s.passed, "blink x2 should pass");
    }

    #[test]
    fn blink_requires_full_closure_cycles() {
        let mut s = LivenessState::new(Challenge::Blink(1));
        // Sustained closed frames count as ONE blink only when re-opened.
        s.update(0.10, 0.0);
        s.update(0.10, 0.0);
        s.update(0.10, 0.0);
        assert!(!s.done, "still closed — should not complete yet");
        s.update(0.30, 0.0); // reopen → blink complete
        assert!(s.done && s.passed);
    }

    #[test]
    fn turn_challenge_requires_sustained_pose() {
        let mut s = LivenessState::new(Challenge::TurnLeft);
        s.update(0.30, 0.0);
        assert!(!s.done);
        s.update(0.30, 0.5);
        assert!(!s.done, "one frame of turn is not enough");
        s.update(0.30, 0.6);
        assert!(!s.done);
        s.update(0.30, 0.7);
        assert!(s.done && s.passed);
    }

    #[test]
    fn turn_right_requires_negative_yaw() {
        let mut s = LivenessState::new(Challenge::TurnRight);
        s.update(0.30, 0.5); // wrong direction
        assert!(!s.done);
        s.update(0.30, -0.5);
        s.update(0.30, -0.6);
        s.update(0.30, -0.7);
        assert!(s.done && s.passed);
    }
}
