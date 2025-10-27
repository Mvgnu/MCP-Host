use std::collections::HashSet;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ed25519_dalek::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::policy::PolicyDecision;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttestationStatus {
    Trusted,
    Untrusted,
    Unknown,
}

impl AttestationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AttestationStatus::Trusted => "trusted",
            AttestationStatus::Untrusted => "untrusted",
            AttestationStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationOutcome {
    pub status: AttestationStatus,
    pub evidence: Option<Value>,
    pub notes: Vec<String>,
    pub attestation_kind: AttestationKind,
    pub freshness_deadline: Option<DateTime<Utc>>,
}

impl AttestationOutcome {
    pub fn trusted(
        kind: AttestationKind,
        evidence: Option<Value>,
        notes: Vec<String>,
        freshness_deadline: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            status: AttestationStatus::Trusted,
            evidence,
            notes,
            attestation_kind: kind,
            freshness_deadline,
        }
    }

    pub fn untrusted(kind: AttestationKind, evidence: Option<Value>, notes: Vec<String>) -> Self {
        Self {
            status: AttestationStatus::Untrusted,
            evidence,
            notes,
            attestation_kind: kind,
            freshness_deadline: None,
        }
    }

    pub fn unknown(kind: AttestationKind, notes: Vec<String>) -> Self {
        Self {
            status: AttestationStatus::Unknown,
            evidence: None,
            notes,
            attestation_kind: kind,
            freshness_deadline: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttestationKind {
    Tpm,
    AmdSevSnp,
    IntelTdx,
    Unknown,
}

impl AttestationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AttestationKind::Tpm => "tpm",
            AttestationKind::AmdSevSnp => "amd-sev-snp",
            AttestationKind::IntelTdx => "intel-tdx",
            AttestationKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NormalizedAttestation {
    pub kind: AttestationKind,
    pub measurement: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub nonce: Option<String>,
    pub claims: Value,
    pub raw_quote: Option<Vec<u8>>,
}

impl NormalizedAttestation {
    pub fn freshness_deadline(&self, max_age: ChronoDuration) -> Option<DateTime<Utc>> {
        self.timestamp.map(|ts| ts + max_age)
    }
}

pub fn detect_kind(evidence: &Value) -> AttestationKind {
    if evidence.get("quote").is_some() {
        AttestationKind::Tpm
    } else if evidence.get("amd_sev_snp").is_some() || evidence.get("sev_report").is_some() {
        AttestationKind::AmdSevSnp
    } else if evidence.get("tdx_quote").is_some() || evidence.get("tdreport").is_some() {
        AttestationKind::IntelTdx
    } else {
        AttestationKind::Unknown
    }
}

pub fn normalize_evidence(evidence: &Value) -> Result<NormalizedAttestation> {
    match detect_kind(evidence) {
        AttestationKind::Tpm => normalize_tpm(evidence),
        AttestationKind::AmdSevSnp => normalize_sev(evidence),
        AttestationKind::IntelTdx => normalize_tdx(evidence),
        AttestationKind::Unknown => Ok(NormalizedAttestation {
            kind: AttestationKind::Unknown,
            measurement: None,
            timestamp: None,
            nonce: None,
            claims: evidence.clone(),
            raw_quote: None,
        }),
    }
}

fn normalize_tpm(evidence: &Value) -> Result<NormalizedAttestation> {
    let quote = evidence.get("quote").context("missing attestation quote")?;
    let report = quote.get("report").context("missing attestation report")?;
    let measurement = report
        .get("measurement")
        .and_then(|v| v.as_str())
        .map(|value| value.trim().to_ascii_lowercase());
    let timestamp = report
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(|ts| DateTime::parse_from_rfc3339(ts))
        .transpose()
        .context("invalid attestation timestamp")?
        .map(|dt| dt.with_timezone(&Utc));

    let nonce = report
        .get("nonce")
        .and_then(|v| v.as_str())
        .map(|value| value.to_string());

    let raw_quote = quote
        .get("raw")
        .or_else(|| evidence.get("raw"))
        .and_then(|value| value.as_str())
        .map(|b64| Base64Engine.decode(b64))
        .transpose()
        .context("invalid attestation raw quote encoding")?;

    Ok(NormalizedAttestation {
        kind: AttestationKind::Tpm,
        measurement,
        timestamp,
        nonce,
        claims: report.clone(),
        raw_quote,
    })
}

fn normalize_sev(evidence: &Value) -> Result<NormalizedAttestation> {
    let report = evidence
        .get("amd_sev_snp")
        .or_else(|| evidence.get("sev_report"))
        .context("missing AMD SEV-SNP report")?;
    let measurement = report
        .get("measurement")
        .and_then(|v| v.as_str())
        .map(|value| value.trim().to_ascii_lowercase());
    let timestamp = report
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(|ts| DateTime::parse_from_rfc3339(ts))
        .transpose()
        .context("invalid SEV-SNP timestamp")?
        .map(|dt| dt.with_timezone(&Utc));

    let raw_quote = report
        .get("raw")
        .and_then(|value| value.as_str())
        .map(|b64| Base64Engine.decode(b64))
        .transpose()
        .context("invalid SEV-SNP raw quote encoding")?;

    let nonce = report
        .get("nonce")
        .and_then(|v| v.as_str())
        .map(|value| value.to_string());

    Ok(NormalizedAttestation {
        kind: AttestationKind::AmdSevSnp,
        measurement,
        timestamp,
        nonce,
        claims: report.clone(),
        raw_quote,
    })
}

fn normalize_tdx(evidence: &Value) -> Result<NormalizedAttestation> {
    let report = evidence
        .get("tdx_quote")
        .or_else(|| evidence.get("tdreport"))
        .context("missing Intel TDX quote")?;
    let measurement = report
        .get("mrseam")
        .or_else(|| report.get("measurement"))
        .and_then(|v| v.as_str())
        .map(|value| value.trim().to_ascii_lowercase());
    let timestamp = report
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(|ts| DateTime::parse_from_rfc3339(ts))
        .transpose()
        .context("invalid Intel TDX timestamp")?
        .map(|dt| dt.with_timezone(&Utc));
    let raw_quote = report
        .get("raw")
        .and_then(|value| value.as_str())
        .map(|b64| Base64Engine.decode(b64))
        .transpose()
        .context("invalid Intel TDX raw quote encoding")?;
    let nonce = report
        .get("report_data")
        .or_else(|| report.get("nonce"))
        .and_then(|v| v.as_str())
        .map(|value| value.to_string());

    Ok(NormalizedAttestation {
        kind: AttestationKind::IntelTdx,
        measurement,
        timestamp,
        nonce,
        claims: report.clone(),
        raw_quote,
    })
}

#[async_trait]
pub trait AttestationVerifier: Send + Sync {
    async fn verify(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        provisioning: &crate::runtime::vm::VmProvisioningResult,
        config: Option<&Value>,
    ) -> Result<AttestationOutcome>;
}

pub struct TpmAttestationVerifier {
    trusted_measurements: HashSet<String>,
    trust_roots: Vec<PublicKey>,
    max_age: Duration,
}

impl TpmAttestationVerifier {
    pub fn new(
        trusted_measurements: HashSet<String>,
        trust_roots: Vec<PublicKey>,
        max_age: Duration,
    ) -> Self {
        Self {
            trusted_measurements,
            trust_roots,
            max_age,
        }
    }

    fn parse_signature(value: &Value) -> Result<Signature> {
        let signature_b64 = value
            .as_str()
            .context("attestation signature must be string")?;
        let decoded = Base64Engine
            .decode(signature_b64)
            .context("invalid attestation signature encoding")?;
        let bytes: [u8; 64] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid signature length"))?;
        let signature =
            Signature::from_bytes(&bytes).context("failed to parse attestation signature")?;
        Ok(signature)
    }

    fn extract_public_keys<'a>(&'a self, evidence: &'a Value) -> Result<Vec<PublicKey>> {
        let mut keys = Vec::new();
        if let Some(key_value) = evidence
            .get("quote")
            .and_then(|quote| quote.get("public_key"))
            .or_else(|| evidence.get("public_key"))
        {
            if let Some(key_str) = key_value.as_str() {
                let decoded = Base64Engine
                    .decode(key_str)
                    .context("invalid attestation public key encoding")?;
                if decoded.len() == 32 {
                    let mut bytes = [0u8; 32];
                    bytes.copy_from_slice(&decoded);
                    if let Ok(verifying) = PublicKey::from_bytes(&bytes) {
                        keys.push(verifying);
                    }
                }
            }
        }

        if keys.is_empty() {
            keys.extend(self.trust_roots.iter().cloned());
        }

        if keys.is_empty() {
            anyhow::bail!("no attestation trust roots configured");
        }

        Ok(keys)
    }
}

#[async_trait]
impl AttestationVerifier for TpmAttestationVerifier {
    async fn verify(
        &self,
        _server_id: i32,
        decision: &PolicyDecision,
        provisioning: &crate::runtime::vm::VmProvisioningResult,
        config: Option<&Value>,
    ) -> Result<AttestationOutcome> {
        let evidence = match provisioning.attestation_evidence.as_ref() {
            Some(evidence) => evidence,
            None => {
                return Ok(AttestationOutcome::untrusted(
                    AttestationKind::Tpm,
                    None,
                    vec!["attestation:missing-evidence".to_string()],
                ))
            }
        };

        let normalized = normalize_evidence(evidence)?;
        let max_age =
            ChronoDuration::from_std(self.max_age).unwrap_or_else(|_| ChronoDuration::minutes(5));

        match normalized.kind {
            AttestationKind::Tpm => {
                let quote = evidence.get("quote").context("missing attestation quote")?;
                let signature_value = quote
                    .get("signature")
                    .context("missing attestation signature")?;
                let measurement = normalized
                    .measurement
                    .clone()
                    .context("missing attestation measurement")?;
                let timestamp = normalized
                    .timestamp
                    .context("missing attestation timestamp")?;

                if Utc::now() - timestamp > max_age {
                    return Ok(AttestationOutcome::untrusted(
                        AttestationKind::Tpm,
                        Some(evidence.clone()),
                        vec!["attestation:stale".to_string()],
                    ));
                }

                if !self.trusted_measurements.contains(&measurement) {
                    return Ok(AttestationOutcome::untrusted(
                        AttestationKind::Tpm,
                        Some(evidence.clone()),
                        vec![format!("attestation:measurement:untrusted:{measurement}")],
                    ));
                }

                if let Some(required_nonce) = config
                    .and_then(|cfg| cfg.get("attestation"))
                    .and_then(|cfg| cfg.get("nonce"))
                    .and_then(|value| value.as_str())
                {
                    let observed_nonce = normalized.nonce.as_deref().unwrap_or_default();
                    if observed_nonce != required_nonce {
                        return Ok(AttestationOutcome::untrusted(
                            AttestationKind::Tpm,
                            Some(evidence.clone()),
                            vec![format!(
                                "attestation:nonce-mismatch:expected:{required_nonce}:actual:{observed_nonce}"
                            )],
                        ));
                    }
                }

                let signature = Self::parse_signature(signature_value)?;
                let message = serde_json::to_vec(&normalized.claims)
                    .context("failed to canonicalize report")?;
                let keys = self.extract_public_keys(evidence)?;

                for key in keys {
                    if key.verify_strict(&message, &signature).is_ok() {
                        let mut notes = vec![
                            format!("attestation:kind:{}", AttestationKind::Tpm.as_str()),
                            format!("attestation:measurement:{measurement}"),
                            format!("attestation:timestamp:{}", timestamp.to_rfc3339()),
                        ];
                        if let Some(nonce_value) = normalized.nonce.as_deref() {
                            notes.push(format!("attestation:nonce:{nonce_value}"));
                        }
                        notes.push(format!("attestation:policy:{}", decision.policy_version));
                        let freshness = normalized.freshness_deadline(max_age);
                        return Ok(AttestationOutcome::trusted(
                            AttestationKind::Tpm,
                            Some(evidence.clone()),
                            notes,
                            freshness,
                        ));
                    }
                }

                Ok(AttestationOutcome::untrusted(
                    AttestationKind::Tpm,
                    Some(evidence.clone()),
                    vec!["attestation:signature-invalid".to_string()],
                ))
            }
            AttestationKind::AmdSevSnp | AttestationKind::IntelTdx => {
                Ok(sev_outcome_from_normalized(
                    decision,
                    normalized,
                    &self.trusted_measurements,
                    max_age,
                ))
            }
            AttestationKind::Unknown => Ok(unsupported_attestation(Some(evidence.clone()))),
        }
    }
}

pub fn unsupported_attestation(evidence: Option<Value>) -> AttestationOutcome {
    let kind = evidence
        .as_ref()
        .map(detect_kind)
        .unwrap_or(AttestationKind::Unknown);
    AttestationOutcome::unknown(
        kind,
        vec![format!("attestation:unsupported-kind:{}", kind.as_str())],
    )
}

pub fn sev_outcome_from_normalized(
    decision: &PolicyDecision,
    normalized: NormalizedAttestation,
    trusted_measurements: &HashSet<String>,
    max_age: ChronoDuration,
) -> AttestationOutcome {
    let measurement = match normalized.measurement.as_ref() {
        Some(measurement) => measurement.clone(),
        None => {
            return AttestationOutcome::untrusted(
                normalized.kind,
                Some(evidence_payload(&normalized)),
                vec!["attestation:missing-measurement".to_string()],
            )
        }
    };

    if !trusted_measurements.contains(&measurement) {
        return AttestationOutcome::untrusted(
            normalized.kind,
            Some(evidence_payload(&normalized)),
            vec![format!("attestation:measurement:untrusted:{measurement}")],
        );
    }

    if let Some(timestamp) = normalized.timestamp {
        if Utc::now() - timestamp > max_age {
            return AttestationOutcome::untrusted(
                normalized.kind,
                Some(evidence_payload(&normalized)),
                vec!["attestation:stale".to_string()],
            );
        }
    }

    let mut notes = vec![
        format!("attestation:kind:{}", normalized.kind.as_str()),
        format!("attestation:measurement:{measurement}"),
    ];
    if let Some(ts) = normalized.timestamp {
        notes.push(format!("attestation:timestamp:{}", ts.to_rfc3339()));
    }
    if let Some(nonce_value) = normalized.nonce.as_deref() {
        notes.push(format!("attestation:nonce:{nonce_value}"));
    }
    notes.push(format!("attestation:policy:{}", decision.policy_version));
    let freshness = normalized.freshness_deadline(max_age);
    AttestationOutcome::trusted(
        normalized.kind,
        Some(evidence_payload(&normalized)),
        notes,
        freshness,
    )
}

fn evidence_payload(normalized: &NormalizedAttestation) -> Value {
    let mut payload = Map::new();
    payload.insert("claims".to_string(), normalized.claims.clone());
    if let Some(raw) = normalized.raw_quote.as_ref() {
        payload.insert("raw".to_string(), json!(Base64Engine.encode(raw)));
    }
    Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attestation_status_strings_are_stable() {
        assert_eq!(AttestationStatus::Trusted.as_str(), "trusted");
        assert_eq!(AttestationStatus::Untrusted.as_str(), "untrusted");
        assert_eq!(AttestationStatus::Unknown.as_str(), "unknown");
    }
}
