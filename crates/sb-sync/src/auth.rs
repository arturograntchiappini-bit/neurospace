use crate::state::{DeviceId, Secret};

const CONTEXT: &[u8] = b"sb-sync-auth-v1";

/// The proof `sender` shows to `receiver` that it knows the group secret.
pub fn proof(secret: &Secret, sender: &DeviceId, receiver: &DeviceId) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(secret);
    hasher.update(CONTEXT).update(sender).update(receiver);
    *hasher.finalize().as_bytes()
}

/// Checks a received proof in constant time.
pub fn verify(secret: &Secret, sender: &DeviceId, receiver: &DeviceId, claimed: &[u8; 32]) -> bool {
    blake3::Hash::from(proof(secret, sender, receiver)) == blake3::Hash::from(*claimed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_bind_proof_to_secret_and_direction() {
        let (s, other) = ([1u8; 32], [2u8; 32]);
        let (a, b, c) = ([10u8; 32], [11u8; 32], [12u8; 32]);
        assert_eq!(proof(&s, &a, &b), proof(&s, &a, &b));
        assert_ne!(proof(&s, &a, &b), proof(&s, &b, &a));
        assert_ne!(proof(&s, &a, &b), proof(&other, &a, &b));
        assert_ne!(proof(&s, &a, &b), proof(&s, &a, &c));
        assert!(verify(&s, &a, &b, &proof(&s, &a, &b)));
        assert!(!verify(&s, &b, &a, &proof(&s, &a, &b)));
    }
}
