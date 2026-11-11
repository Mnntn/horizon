//! Load-owned raw payload registrations. No connection state lives here.
#[cfg(test)]
#[path = "payloads/tests.rs"]
mod tests;
use super::*;
use crate::{
    HandlerThread, IPayloadHandler, PayloadDeclaration, PayloadDirection, PayloadId,
    PayloadProtocol,
};

#[derive(Clone)]
struct Registered {
    declaration: PayloadDeclaration,
    owner: &'static str,
    owner_index: usize,
    server: Option<Arc<dyn IPayloadHandler>>,
    client: Option<Arc<dyn IPayloadHandler>>,
    client_thread: HandlerThread,
}

// no wire: frozen process-local registration storage contains callback trait objects and owning-mod identities.
/// Immutable payload registry for engine connection setup and dispatch.
/// It is absent entirely when no mod registers a payload.
#[derive(Clone, Default)]
pub struct Payloads {
    phases: [HashMap<String, Registered>; 2],
}

impl Payloads {
    /// Iterate normalized single-protocol declarations for negotiation.
    pub fn declarations(&self) -> impl Iterator<Item = &PayloadDeclaration> {
        self.phases
            .iter()
            .flat_map(|phase| phase.values().map(|entry| &entry.declaration))
    }
    /// Resolve the sole receiving owner. A disabled owner or absent callback has
    /// no handler; the connection must report failure, never silently skip it.
    pub fn handler(
        &self,
        id: &str,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
    ) -> Option<(Arc<dyn IPayloadHandler>, HandlerThread)> {
        let index = match protocol {
            PayloadProtocol::Configuration => 0,
            PayloadProtocol::Play => 1,
            PayloadProtocol::Common => return None,
        };
        let entry = self.phases[index].get(id)?;
        if !crate::mod_index_on(entry.owner_index) {
            return None;
        }
        match direction {
            PayloadDirection::ToClient => entry
                .client
                .clone()
                .map(|handler| (handler, entry.client_thread)),
            PayloadDirection::ToServer => entry
                .server
                .clone()
                .map(|handler| (handler, entry.declaration.thread)),
            PayloadDirection::Bidirectional => None,
        }
    }
    /// Validate the client-only handler requirement before a client starts its
    /// connection. Dedicated servers do not require unused client callbacks.
    pub fn validate_client_handlers(&self) -> Result<(), Vec<String>> {
        let missing: Vec<_> = self
            .phases
            .iter()
            .flat_map(|phase| phase.values())
            .filter(|entry| {
                entry.declaration.direction != PayloadDirection::ToServer && entry.client.is_none()
            })
            .map(|entry| {
                format!(
                    "{} has no client payload handler for {} in {:?}",
                    entry.owner, entry.declaration.id.0, entry.declaration.protocol
                )
            })
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

/// The frozen process registry, shared rather than copied into every connection.
#[doc(hidden)]
pub fn installed_payloads() -> Option<Arc<Payloads>> {
    installed().and_then(|installed| installed.keys.payloads.clone())
}

/// Load-only operations backing the public payload registration views.
pub trait PayloadsRegistrar {
    /// Register a declaration and its receiving callbacks. Invalid declarations
    /// join the registrar's startup refusal list, naming the owning mod.
    fn add_payload(
        &mut self,
        declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    );
    /// Attach the client callback to this owner's already declared clientbound
    /// entries in every protocol. Duplicates and unknown ids refuse startup.
    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    );
}

fn indices(protocol: PayloadProtocol) -> &'static [usize] {
    match protocol {
        PayloadProtocol::Configuration => &[0],
        PayloadProtocol::Play => &[1],
        PayloadProtocol::Common => &[0, 1],
    }
}

impl PayloadsRegistrar for PartRegistrar {
    fn add_payload(
        &mut self,
        mut declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) {
        let refused = (|| {
            if !declaration.id.0.contains(':') {
                return Err("payload ids must be explicitly namespaced".to_owned());
            }
            declaration.id.0 = self.owned_declaration_id(&declaration.id.0)?;
            declaration.version = declaration.version.trim().to_owned();
            if declaration.version.is_empty() {
                return Err("payload version must not be blank".to_owned());
            }
            if declaration.direction != PayloadDirection::ToClient && server.is_none() {
                return Err("serverbound payload requires a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToClient && server.is_some() {
                return Err("clientbound-only payload has a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToServer && client.is_some() {
                return Err("serverbound-only payload has a client handler".to_owned());
            }
            if let Some(payloads) = &self.parts.payloads {
                for &index in indices(declaration.protocol) {
                    if payloads.phases[index].contains_key(&declaration.id.0) {
                        return Err(format!(
                            "duplicate payload registration for {} in protocol {index}",
                            declaration.id.0
                        ));
                    }
                }
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} registered payload {}: {reason}",
                self.mod_id, declaration.id.0
            ));
            return;
        }
        let payloads = Arc::make_mut(
            self.parts
                .payloads
                .get_or_insert_with(|| Arc::new(Payloads::default())),
        );
        for &index in indices(declaration.protocol) {
            let mut normalized = declaration.clone();
            normalized.protocol = if index == 0 {
                PayloadProtocol::Configuration
            } else {
                PayloadProtocol::Play
            };
            payloads.phases[index].insert(
                declaration.id.0.clone(),
                Registered {
                    declaration: normalized,
                    owner: self.mod_id,
                    owner_index: self.mod_index,
                    server: server.clone(),
                    client: client.clone(),
                    client_thread: declaration.thread,
                },
            );
        }
    }

    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    ) {
        let Some(payloads) = self.parts.payloads.as_mut() else {
            self.refused.push(format!(
                "{} supplied a client handler for unknown payload {}",
                self.mod_id, id.0
            ));
            return;
        };
        let mut found = false;
        for phase in &payloads.phases {
            if let Some(entry) = phase.get(&id.0) {
                if entry.owner != self.mod_id {
                    self.refused.push(format!(
                        "{} cannot attach a client handler to {} owned by {}",
                        self.mod_id, id.0, entry.owner
                    ));
                    return;
                }
                if entry.declaration.direction != PayloadDirection::ToServer {
                    found = true;
                    if entry.client.is_some() {
                        self.refused.push(format!(
                            "{} registered a second client payload handler for {}",
                            self.mod_id, id.0
                        ));
                        return;
                    }
                }
            }
        }
        if !found {
            self.refused.push(format!(
                "{} supplied a client handler for unknown or serverbound-only payload {}",
                self.mod_id, id.0
            ));
            return;
        }
        for phase in &mut Arc::make_mut(payloads).phases {
            if let Some(entry) = phase.get_mut(&id.0)
                && entry.declaration.direction != PayloadDirection::ToServer
            {
                entry.client = Some(handler.clone());
                entry.client_thread = thread;
            }
        }
    }
}
