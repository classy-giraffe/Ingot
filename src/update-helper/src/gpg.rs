//! GPG authentication: verify detached signatures over
//! `manifest.json` and `SHA256SUMS` against the project keyring.
//!
//! Trust semantics match systemd-pull's `--trust-model=always`: the
//! signing key must be *present in the keyring* and the signature
//! cryptographically valid; no trust chain or expiry policy beyond
//! the key's own validity is evaluated.

use anyhow::Context;
use sequoia_openpgp as openpgp;
use openpgp::cert::CertParser;
use openpgp::parse::stream::{
    DetachedVerifierBuilder, MessageLayer, VerificationError, VerificationHelper,
};
use openpgp::parse::Parse;
use openpgp::policy::StandardPolicy;
use openpgp::{Cert, KeyHandle};

/// Why verification failed.
#[derive(Debug)]
pub enum GpgError {
    /// The signing key is not in the keyring.
    UnknownKey { issuer: String },
    /// The signature is invalid (tampered data, bad key state, or a
    /// malformed signature).
    Invalid { reason: String },
}

impl std::fmt::Display for GpgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpgError::UnknownKey { issuer } => {
                write!(f, "signing key not in keyring (issuer {issuer})")
            }
            GpgError::Invalid { reason } => f.write_str(reason),
        }
    }
}

/// A loaded public keyring (the project key shipped in the slot's
/// vendor keyring, or a test keyring).
pub struct Keyring {
    certs: Vec<Cert>,
}

impl Keyring {
    /// Loads a keyring file (binary or ASCII-armored, one or more
    /// keys).
    pub fn load(path: &str) -> anyhow::Result<Keyring> {
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read keyring {path}"))?;
        let certs: Vec<Cert> = CertParser::from_bytes(&bytes[..])
            .with_context(|| format!("cannot parse keyring {path}"))?
            .collect::<openpgp::Result<Vec<_>>>()
            .with_context(|| format!("cannot parse keyring {path}"))?;
        if certs.is_empty() {
            anyhow::bail!("keyring {path} contains no keys");
        }
        Ok(Keyring { certs })
    }

    /// Verifies the detached signature `sig` over `data`, requiring
    /// the signing key to be present in this keyring.
    pub fn verify(&self, sig: &[u8], data: &[u8]) -> Result<(), GpgError> {
        let policy = StandardPolicy::new();
        let mut verifier = DetachedVerifierBuilder::from_bytes(sig)
            .map_err(|e| GpgError::Invalid {
                reason: format!("malformed signature: {e}"),
            })?
            .with_policy(&policy, None, KeyringHelper { certs: &self.certs, outcome: None })
            .map_err(|e| GpgError::Invalid {
                reason: format!("malformed signature: {e}"),
            })?;
        verifier
            .verify_bytes(data)
            .map_err(|e| GpgError::Invalid {
                reason: format!("verification aborted: {e}"),
            })?;
        verifier
            .into_helper()
            .outcome
            .unwrap_or(Err(GpgError::Invalid {
                reason: "signature verification produced no outcome".to_string(),
            }))
    }
}

/// The verification helper: serves keyring certs to the verifier and
/// records the outcome (one signature group for a detached
/// signature).
struct KeyringHelper<'a> {
    certs: &'a [Cert],
    outcome: Option<Result<(), GpgError>>,
}

impl VerificationHelper for KeyringHelper<'_> {
    fn get_certs(&mut self, ids: &[KeyHandle]) -> openpgp::Result<Vec<Cert>> {
        let mut out = Vec::new();
        for id in ids {
            for cert in self.certs {
                if cert_contains_key(cert, id) {
                    out.push(cert.clone());
                }
            }
        }
        Ok(out)
    }

    fn check(&mut self, structure: openpgp::parse::stream::MessageStructure) -> openpgp::Result<()> {
        for layer in structure.iter() {
            if let MessageLayer::SignatureGroup { results } = layer {
                let mut any_good = false;
                let mut first_err: Option<GpgError> = None;
                for result in results {
                    match result {
                        Ok(_) => any_good = true,
                        Err(e) => {
                            if first_err.is_none() {
                                first_err = Some(classify(&e));
                            }
                        }
                    }
                }
                if results.is_empty() {
                    first_err = Some(GpgError::Invalid {
                        reason: "no signatures".into(),
                    });
                }
                // Any signature good under the keyring suffices
                // (matches gpg --verify: a document signed by any key
                // in the keyring is authenticated; rotation keeps old
                // keys in the keyring).
                self.outcome = Some(if any_good {
                    Ok(())
                } else {
                    Err(first_err.expect("an error was recorded"))
                });
            }
        }
        Ok(())
    }
}

/// Whether the certificate contains a key (primary or subkey) that
/// matches the handle.
fn cert_contains_key(cert: &Cert, id: &KeyHandle) -> bool {
    match id {
        KeyHandle::Fingerprint(fp) => {
            cert.keys().any(|ka| ka.key().fingerprint() == *fp)
        }
        KeyHandle::KeyID(kid) => cert
            .keys()
            .any(|ka| openpgp::KeyID::from(ka.key().fingerprint()) == *kid),
    }
}

/// Maps a sequoia verification failure to a clear error.
fn classify(e: &VerificationError<'_>) -> GpgError {
    match e {
        VerificationError::MissingKey { sig } => {
            let issuer = sig
                .get_issuers()
                .into_iter()
                .next()
                .map(|h| h.to_string())
                .unwrap_or_else(|| "unknown".into());
            GpgError::UnknownKey { issuer }
        }
        // BadSignature, BadKey, UnboundKey, MalformedSignature,
        // UnknownSignature: the signature's own Display is already a
        // clear, actionable message.
        other => GpgError::Invalid {
            reason: other.to_string(),
        },
    }
}
