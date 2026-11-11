//! Bounded configuration-only metadata negotiation. Application bodies never use this codec.
use crate::buf::{Codec, Ident, Reader, Writer};
use std::collections::BTreeMap;

pub const CHANNEL: &str = "horizon:payloads";
pub const MAX_RECORDS: usize = 4096;
pub const MAX_MANIFEST_BYTES: usize = 1_048_576;
const MAX_STRING: usize = 32767;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Configuration,
    Play,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationPhase {
    Configuration,
    Play,
    Common,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    ToClient,
    ToServer,
    Bidirectional,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Codec(String),
    MetadataTooLarge,
    TooManyRecords,
    InvalidFlags,
    InvalidId(String),
    InvalidVersion,
    Duplicate {
        phase: Phase,
        id: String,
    },
    TrailingBytes,
    UnexpectedOffer,
    Frozen,
    Required {
        phase: Phase,
        id: String,
        reason: Mismatch,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mismatch {
    Missing,
    Version,
    Direction,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Required { phase, id, reason } => {
                write!(f, "required payload {id} ({phase:?}): {reason:?}")
            }
            Self::Duplicate { phase, id } => write!(f, "duplicate payload {id} ({phase:?})"),
            Self::InvalidId(id) => write!(f, "invalid payload id {id}"),
            Self::Codec(reason) => write!(f, "payload manifest codec: {reason}"),
            other => write!(f, "payload negotiation: {other:?}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<crate::Error> for Error {
    fn from(e: crate::Error) -> Self {
        Self::Codec(e.to_string())
    }
}

/// Validated immutable phase/id declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    phase: Phase,
    id: String,
    flow: Flow,
    version: String,
    optional: bool,
}
impl Entry {
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn flow(&self) -> Flow {
        self.flow
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn optional(&self) -> bool {
        self.optional
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<Entry>,
}
impl Manifest {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn builder() -> ManifestBuilder {
        ManifestBuilder::default()
    }
}
#[derive(Default)]
pub struct ManifestBuilder {
    entries: BTreeMap<(Phase, String), Entry>,
    record_bytes: usize,
}
fn varint_bytes(n: usize) -> usize {
    if n < 128 {
        1
    } else if n < 16384 {
        2
    } else {
        3
    }
}
impl ManifestBuilder {
    pub fn register(
        &mut self,
        phase: RegistrationPhase,
        id: &str,
        flow: Flow,
        version: &str,
        optional: bool,
    ) -> Result<(), Error> {
        if id.encode_utf16().count() > MAX_STRING {
            return Err(Error::InvalidId(id.into()));
        }
        let mut w = Writer::new();
        w.string(id);
        let parsed =
            Ident::read(&mut Reader::new(&w.data)).map_err(|_| Error::InvalidId(id.into()))?;
        if parsed != id
            || id
                .split_once(':')
                .is_none_or(|(n, p)| n.is_empty() || p.is_empty())
        {
            return Err(Error::InvalidId(id.into()));
        }
        if version.trim().is_empty() || version.encode_utf16().count() > MAX_STRING {
            return Err(Error::InvalidVersion);
        }
        let phases: &[Phase] = match phase {
            RegistrationPhase::Configuration => &[Phase::Configuration],
            RegistrationPhase::Play => &[Phase::Play],
            RegistrationPhase::Common => &[Phase::Configuration, Phase::Play],
        };
        if self.entries.len() + phases.len() > MAX_RECORDS {
            return Err(Error::TooManyRecords);
        }
        let record_bytes = self.record_bytes
            + phases.len()
                * (3 + varint_bytes(id.len())
                    + id.len()
                    + varint_bytes(version.len())
                    + version.len());
        if 1 + varint_bytes(self.entries.len() + phases.len()) + record_bytes > MAX_MANIFEST_BYTES {
            return Err(Error::MetadataTooLarge);
        }
        for &phase in phases {
            if self.entries.contains_key(&(phase, id.into())) {
                return Err(Error::Duplicate {
                    phase,
                    id: id.into(),
                });
            }
        }
        self.record_bytes = record_bytes;
        for &phase in phases {
            self.entries.insert(
                (phase, id.into()),
                Entry {
                    phase,
                    id: id.into(),
                    flow,
                    version: version.into(),
                    optional,
                },
            );
        }
        Ok(())
    }
    pub fn finish(self) -> Result<Manifest, Error> {
        let manifest = Manifest {
            entries: self.entries.into_values().collect(),
        };
        Ok(manifest)
    }
}

/// Tags: Query = 0; Offer = 1, then VarInt count and phase/id/flow/version/optional records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Query,
    Offer(Manifest),
}
impl Control {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut w = Writer::new();
        match self {
            Self::Query => w.u8(0),
            Self::Offer(m) => {
                w.u8(1);
                w.varint(m.entries.len() as i32);
                for e in &m.entries {
                    w.u8(e.phase as u8);
                    w.string(&e.id);
                    w.u8(e.flow as u8);
                    w.string(&e.version);
                    w.bool(e.optional);
                    if w.data.len() > MAX_MANIFEST_BYTES {
                        return Err(Error::MetadataTooLarge);
                    }
                }
            }
        }
        Ok(w.data)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        // Bound the complete borrowed input before allocating any metadata.
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::MetadataTooLarge);
        }
        let mut r = Reader::new(bytes);
        let message = match r.u8()? {
            0 => Self::Query,
            1 => {
                let count = r.varint()?;
                if count < 0 || count as usize > MAX_RECORDS {
                    return Err(Error::TooManyRecords);
                }
                let mut b = Manifest::builder();
                for _ in 0..count {
                    let phase = match r.u8()? {
                        0 => RegistrationPhase::Configuration,
                        1 => RegistrationPhase::Play,
                        _ => return Err(Error::InvalidFlags),
                    };
                    let id = r.string(MAX_STRING)?;
                    let flow = match r.u8()? {
                        0 => Flow::ToClient,
                        1 => Flow::ToServer,
                        2 => Flow::Bidirectional,
                        _ => return Err(Error::InvalidFlags),
                    };
                    let version = r.string(MAX_STRING)?;
                    let optional = match r.u8()? {
                        0 => false,
                        1 => true,
                        _ => return Err(Error::InvalidFlags),
                    };
                    b.register(phase, &id, flow, &version, optional)?;
                }
                Self::Offer(b.finish()?)
            }
            _ => return Err(Error::InvalidFlags),
        };
        if r.remaining() != 0 {
            return Err(Error::TrailingBytes);
        }
        Ok(message)
    }
}

/// One instance per configuration epoch. Errors are admission failures; discard the instance.
pub struct Negotiation {
    local: Manifest,
    offered: bool,
    frozen: Option<Vec<Entry>>,
}
impl Negotiation {
    pub fn new(local: Manifest) -> Self {
        Self {
            local,
            offered: false,
            frozen: None,
        }
    }
    /// Call once at setup. Empty installations emit no control traffic.
    pub fn initial(&self) -> Option<Control> {
        (!self.local.entries.is_empty()).then_some(Control::Query)
    }
    pub fn accepted(&self) -> Option<&[Entry]> {
        self.frozen.as_deref()
    }
    pub fn receive(&mut self, control: Control) -> Result<Option<Control>, Error> {
        if self.frozen.is_some() {
            return Err(match control {
                Control::Offer(_) => Error::UnexpectedOffer,
                Control::Query => Error::Frozen,
            });
        }
        match control {
            Control::Query => Ok(self.offer()),
            Control::Offer(peer) => {
                let accepted = intersect(&self.local, &peer)?;
                let reply = self.offer();
                self.frozen = Some(accepted);
                Ok(reply)
            }
        }
    }
    fn offer(&mut self) -> Option<Control> {
        if self.offered {
            None
        } else {
            self.offered = true;
            Some(Control::Offer(self.local.clone()))
        }
    }
    /// Invoke only at the real known-packs/FinishConfiguration ordering barrier.
    pub fn finish_without_offer(&mut self) -> Result<(), Error> {
        if self.frozen.is_none() {
            self.frozen = Some(intersect(&self.local, &Manifest::default())?);
        }
        Ok(())
    }
}
fn intersect(local: &Manifest, peer: &Manifest) -> Result<Vec<Entry>, Error> {
    let l: BTreeMap<_, _> = local
        .entries
        .iter()
        .map(|e| ((e.phase, e.id.as_str()), e))
        .collect();
    let p: BTreeMap<_, _> = peer
        .entries
        .iter()
        .map(|e| ((e.phase, e.id.as_str()), e))
        .collect();
    let mut accepted = Vec::new();
    for (key, a) in &l {
        let b = p.get(key);
        let mismatch = match b {
            None => Some(Mismatch::Missing),
            Some(b) if a.version != b.version => Some(Mismatch::Version),
            Some(b) if a.flow != b.flow => Some(Mismatch::Direction),
            _ => None,
        };
        if let Some(reason) = mismatch {
            if !a.optional || b.is_some_and(|b| !b.optional) {
                return Err(Error::Required {
                    phase: a.phase,
                    id: a.id.clone(),
                    reason,
                });
            }
        } else {
            accepted.push((*a).clone());
        }
    }
    for (key, b) in &p {
        if !l.contains_key(key) && !b.optional {
            return Err(Error::Required {
                phase: b.phase,
                id: b.id.clone(),
                reason: Mismatch::Missing,
            });
        }
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests;
