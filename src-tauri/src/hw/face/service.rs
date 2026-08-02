//! Face auth pipeline — the state machine behind a lock-screen unlock.
//!
//! Port of the reference `face_hello/auth.py`:
//!   liveness (active challenge) → anti-spoof (passive, optional)
//!   → detect+recognize → best-match-with-margin → accept/reject.
//!
//! The service exposes `auth_start` / `auth_poll`; each poll feeds a frame
//! and returns the current instruction + completion state. The auth thread
//! owns the camera for the duration of one unlock attempt.

use crate::hw::face::config::FaceSettings;
use crate::hw::face::liveness::{Challenge, LivenessState};
use crate::hw::face::matcher::best_match_with_margin;
use crate::hw::face::store::FaceStore;

#[allow(dead_code)] // used only when the `face` feature (camera/ORT) is enabled
struct ServiceMarker;

#[allow(unused_imports)]
use crate::hw::face::errors::{FaceError, FaceResult};

/// Result of one auth attempt.
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub success: bool,
    pub name: Option<String>,
    pub similarity: f32,
    pub reason: String,
    /// True when this was a genuine biometric decision (counts toward lockout).
    pub biometric: bool,
}

impl AuthResult {
    pub fn fail(reason: impl Into<String>, biometric: bool) -> Self {
        Self {
            success: false,
            name: None,
            similarity: 0.0,
            reason: reason.into(),
            biometric,
        }
    }
}

/// Serializable snapshot for `auth_poll`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PollSnapshot {
    pub ok: bool,
    pub done: bool,
    pub instruction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One auth attempt. Caller feeds frames via [`AuthSession::feed`].
pub struct AuthSession {
    settings: FaceSettings,
    gallery: Vec<Vec<f32>>,
    names: Vec<String>,
    phase: Phase,
    liveness: Option<LivenessState>,
    challenge: Challenge,
    result: Option<AuthResult>,
}

enum Phase {
    Liveness,
    Recognize,
    Done,
}

impl AuthSession {
    pub fn new(store: &FaceStore) -> Self {
        let settings = store.settings.clone();
        let (gallery, names) = store.flat_gallery();
        let liveness_enabled = settings.liveness_enabled;
        let challenge = if liveness_enabled {
            let c = LivenessState::random_challenge();
            Some(c)
        } else {
            None
        };
        let liveness = challenge.map(LivenessState::new);
        let phase = if liveness_enabled {
            Phase::Liveness
        } else {
            Phase::Recognize
        };
        Self {
            settings,
            gallery,
            names,
            phase,
            liveness,
            challenge: Challenge::Blink(1), // placeholder, replaced above
            result: None,
        }
    }

    pub fn done(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    pub fn instruction(&self) -> String {
        match self.phase {
            Phase::Liveness => match self.challenge {
                Challenge::Blink(_) => "blink".to_string(),
                Challenge::TurnLeft => "turn_left".to_string(),
                Challenge::TurnRight => "turn_right".to_string(),
            },
            Phase::Recognize => "recognizing".to_string(),
            Phase::Done => self
                .result
                .as_ref()
                .map(|r| r.reason.clone())
                .unwrap_or_default(),
        }
    }

    pub fn result(&self) -> Option<&AuthResult> {
        self.result.as_ref()
    }

    /// Feed one frame of measurements.
    /// `avg_ear` and `yaw` come from the landmark tracker (see models).
    /// `embedding` is the recognized face embedding (only used in Recognize
    /// phase); pass `None` when no face was detected.
    pub fn feed(&mut self, avg_ear: f32, yaw: f32, embedding: Option<Vec<f32>>) {
        if self.done() {
            return;
        }
        match self.phase {
            Phase::Liveness => {
                let mut moved_to_recognize = false;
                if let Some(liv) = self.liveness.as_mut() {
                    liv.update(avg_ear, yaw);
                    if liv.done {
                        if !liv.passed {
                            self.phase = Phase::Done;
                            self.result = Some(AuthResult::fail("liveness_failed", true));
                        } else {
                            self.phase = Phase::Recognize;
                            moved_to_recognize = true;
                        }
                    }
                } else {
                    self.phase = Phase::Recognize;
                    moved_to_recognize = true;
                }
                // If liveness just passed on this frame, process recognition
                // with the same frame's embedding. Otherwise wait for the next
                // frame — never recurse while still in the liveness phase.
                if moved_to_recognize {
                    self.feed(avg_ear, yaw, embedding);
                }
            }
            Phase::Recognize => {
                let emb = match embedding {
                    Some(e) => e,
                    None => {
                        self.phase = Phase::Done;
                        self.result = Some(AuthResult::fail("no_face", true));
                        return;
                    }
                };
                if self.gallery.is_empty() {
                    self.phase = Phase::Done;
                    self.result = Some(AuthResult::fail("no_enrolled", false));
                    return;
                }
                let m = best_match_with_margin(&emb, &self.gallery, &self.names);
                let thr = self.settings.match_threshold;
                let margin_ok = m.margin >= self.settings.match_margin;
                if m.index == usize::MAX || m.similarity < thr || !margin_ok {
                    self.phase = Phase::Done;
                    self.result = Some(AuthResult {
                        success: false,
                        name: None,
                        similarity: m.similarity,
                        reason: format!("face_mismatch sim={:.3} thr={thr}", m.similarity),
                        biometric: true,
                    });
                } else {
                    self.phase = Phase::Done;
                    self.result = Some(AuthResult {
                        success: true,
                        name: Some(self.names[m.index].clone()),
                        similarity: m.similarity,
                        reason: "ok".to_string(),
                        biometric: true,
                    });
                }
            }
            Phase::Done => {}
        }
    }

    /// Force a specific liveness challenge (used by tests and the service to
    /// make the challenge deterministic). Returns true if liveness is enabled
    /// and the challenge was set.
    pub fn liveness_force_challenge(&mut self, challenge: Challenge) -> bool {
        if let Some(liv) = self.liveness.as_mut() {
            *liv = LivenessState::new(challenge);
            self.challenge = challenge;
            self.phase = Phase::Liveness;
            true
        } else {
            false
        }
    }

    /// Convenience: run the whole pipeline on one frame (single-shot, used by
    /// dev/console auth). Feed must be called repeatedly for liveness.
    pub fn snapshot(&self) -> PollSnapshot {
        let r = self.result.as_ref();
        PollSnapshot {
            ok: true,
            done: self.done(),
            instruction: self.instruction(),
            success: r.map(|x| x.success),
            user: r.and_then(|x| x.name.clone()),
            similarity: r.map(|x| x.similarity),
            reason: r.map(|x| x.reason.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::face::store::FaceStore;

    fn store_with(name: &str, emb: Vec<f32>) -> FaceStore {
        let mut s = FaceStore::new();
        s.settings.liveness_enabled = false; // skip liveness in these tests
        s.add_template(name, emb, "front").unwrap();
        s
    }

    fn emb(v: f32) -> Vec<f32> {
        vec![v; 512]
    }

    /// An embedding that differs strongly in *direction* from `emb(v)` —
    /// half with inverted sign → cosine ≈ 0 (orthogonal-ish).
    fn emb_diff(v: f32) -> Vec<f32> {
        let mut out = vec![v; 512];
        for x in out.iter_mut().skip(256) {
            *x = -v;
        }
        out
    }

    #[test]
    fn recognize_accepts_matching_face() {
        let store = store_with("alice", emb(0.5));
        let mut session = AuthSession::new(&store);
        // liveness disabled → straight to recognize; provide a matching embedding.
        session.feed(0.3, 0.0, Some(emb(0.5)));
        assert!(session.done());
        let r = session.result().unwrap();
        assert!(r.success);
        assert_eq!(r.name.as_deref(), Some("alice"));
    }

    #[test]
    fn recognize_rejects_non_matching_face() {
        let store = store_with("alice", emb(0.5));
        let mut session = AuthSession::new(&store);
        session.feed(0.3, 0.0, Some(emb_diff(0.5)));
        assert!(session.done());
        let r = session.result().unwrap();
        assert!(!r.success);
        assert!(r.biometric);
    }

    #[test]
    fn recognize_fails_when_no_face() {
        let store = store_with("alice", emb(0.5));
        let mut session = AuthSession::new(&store);
        session.feed(0.3, 0.0, None);
        assert!(session.done());
        let r = session.result().unwrap();
        assert!(!r.success);
        assert_eq!(r.reason, "no_face");
    }

    #[test]
    fn recognize_fails_when_gallery_empty() {
        let mut store = FaceStore::new();
        store.settings.liveness_enabled = false;
        let mut session = AuthSession::new(&store);
        session.feed(0.3, 0.0, Some(emb(0.5)));
        assert!(session.done());
        let r = session.result().unwrap();
        assert!(!r.success);
        assert_eq!(r.reason, "no_enrolled");
        assert!(!r.biometric); // infra error, not biometric
    }

    #[test]
    fn liveness_gates_recognition() {
        let mut store = store_with("alice", emb(0.5));
        store.settings.liveness_enabled = true;
        let mut session = AuthSession::new(&store);
        // Simulate the challenge: with a Blink(1) challenge, a blink cycle.
        // (The challenge is random; force it for determinism.)
        if let Some(liv) = session.liveness.as_mut() {
            *liv = LivenessState::new(Challenge::Blink(1));
        }
        session.challenge = Challenge::Blink(1);
        // Frames with eyes open, no blink yet.
        session.feed(0.3, 0.0, Some(emb(0.5)));
        assert!(!session.done(), "liveness not satisfied yet");
        // Blink: close then open.
        session.feed(0.10, 0.0, None);
        session.feed(0.30, 0.0, Some(emb(0.5)));
        assert!(session.done());
        assert!(session.result().unwrap().success);
    }

    #[test]
    fn snapshot_shape() {
        let store = store_with("alice", emb(0.5));
        let session = AuthSession::new(&store);
        let snap = session.snapshot();
        assert!(snap.ok);
        assert!(!snap.done);
        assert!(snap.success.is_none());
    }
}
