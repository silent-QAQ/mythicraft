use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    evaluate_experience_capability, AssetManifest, AudioClientGate, AudioDecision, AudioError,
    ClientCapability, ComponentRevisionGate, ExperienceDecision, ExperienceError, ProtocolMessage,
    RevisionDecision,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSessionPhase {
    AwaitingCapabilities,
    Ready,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEffect {
    CapabilitiesAccepted,
    CapabilitiesRejected {
        reasons: Vec<String>,
    },
    PageOpened {
        page_id: String,
        version: u64,
    },
    PageDegraded {
        page_id: String,
        decision: ExperienceDecision,
    },
    PageUpdated {
        page_id: String,
        version: u64,
    },
    PageClosed {
        page_id: String,
    },
    ManifestInstalled {
        manifest_hash: String,
    },
    AssetResultReceived {
        resource_id: String,
    },
    AudioEvaluated {
        event_id: String,
        decision: AudioDecision,
    },
    AudioStopped {
        event_id: String,
    },
    TransientComponent {
        component_id: String,
        decision: ExperienceDecision,
    },
    VersionedComponent {
        component_id: String,
        revision: u64,
        revision_decision: RevisionDecision,
        render_decision: ExperienceDecision,
    },
}

#[derive(Debug)]
pub struct ClientSession {
    phase: ClientSessionPhase,
    accepted_capabilities: BTreeSet<ClientCapability>,
    pages: BTreeMap<String, u64>,
    manifest: Option<AssetManifest>,
    audio_gate: AudioClientGate,
    revision_gate: ComponentRevisionGate,
}

impl ClientSession {
    pub fn new(max_audio_events: usize, audio_window_ms: u64) -> Result<Self, ClientSessionError> {
        Ok(Self {
            phase: ClientSessionPhase::AwaitingCapabilities,
            accepted_capabilities: BTreeSet::new(),
            pages: BTreeMap::new(),
            manifest: None,
            audio_gate: AudioClientGate::new(max_audio_events, audio_window_ms)?,
            revision_gate: ComponentRevisionGate::default(),
        })
    }

    pub fn phase(&self) -> ClientSessionPhase {
        self.phase
    }

    pub fn accepted_capabilities(&self) -> &BTreeSet<ClientCapability> {
        &self.accepted_capabilities
    }

    pub fn active_page_version(&self, page_id: &str) -> Option<u64> {
        self.pages.get(page_id).copied()
    }

    pub fn apply(
        &mut self,
        message: ProtocolMessage,
        now_unix_ms: u64,
        available_resources: &BTreeSet<String>,
    ) -> Result<ClientEffect, ClientSessionError> {
        if self.phase == ClientSessionPhase::AwaitingCapabilities {
            return self.apply_capabilities(message);
        }
        if self.phase == ClientSessionPhase::Rejected {
            return Err(ClientSessionError::SessionRejected);
        }

        match message {
            ProtocolMessage::Capabilities(_) => Err(ClientSessionError::DuplicateCapabilities),
            ProtocolMessage::Hello(_)
            | ProtocolMessage::UiAction(_)
            | ProtocolMessage::AssetRequest(_) => Err(ClientSessionError::WrongDirection),
            ProtocolMessage::UiOpen(open) => {
                if let Some(missing) = open
                    .required_capabilities
                    .iter()
                    .find(|capability| !self.accepted_capabilities.contains(capability))
                {
                    return Ok(ClientEffect::PageDegraded {
                        page_id: open.page_id,
                        decision: evaluate_experience_capability(
                            missing.clone(),
                            &self.accepted_capabilities,
                            true,
                        ),
                    });
                }
                self.pages.insert(open.page_id.clone(), open.page_version);
                Ok(ClientEffect::PageOpened {
                    page_id: open.page_id,
                    version: open.page_version,
                })
            }
            ProtocolMessage::UiUpdate(update) => {
                let current = self
                    .pages
                    .get(&update.page_id)
                    .copied()
                    .ok_or_else(|| ClientSessionError::PageNotOpen(update.page_id.clone()))?;
                if current != update.expected_page_version {
                    return Err(ClientSessionError::PageVersionMismatch {
                        page_id: update.page_id,
                        expected: current,
                        received: update.expected_page_version,
                    });
                }
                self.pages
                    .insert(update.page_id.clone(), update.page_version);
                Ok(ClientEffect::PageUpdated {
                    page_id: update.page_id,
                    version: update.page_version,
                })
            }
            ProtocolMessage::UiClose(close) => {
                let current = self
                    .pages
                    .get(&close.page_id)
                    .copied()
                    .ok_or_else(|| ClientSessionError::PageNotOpen(close.page_id.clone()))?;
                if current != close.page_version {
                    return Err(ClientSessionError::PageVersionMismatch {
                        page_id: close.page_id,
                        expected: current,
                        received: close.page_version,
                    });
                }
                self.pages.remove(&close.page_id);
                Ok(ClientEffect::PageClosed {
                    page_id: close.page_id,
                })
            }
            ProtocolMessage::AssetManifest(manifest) => {
                let manifest_hash = manifest.manifest_hash.clone();
                self.manifest = Some(manifest);
                Ok(ClientEffect::ManifestInstalled { manifest_hash })
            }
            ProtocolMessage::AssetResult(result) => Ok(ClientEffect::AssetResultReceived {
                resource_id: result.resource_id,
            }),
            ProtocolMessage::AudioPlay(event) => {
                let in_manifest = self
                    .manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.find(&event.sound_id).is_some());
                let available = in_manifest && available_resources.contains(&event.sound_id);
                let decision = self.audio_gate.evaluate(&event, now_unix_ms, available)?;
                Ok(ClientEffect::AudioEvaluated {
                    event_id: event.event_id,
                    decision,
                })
            }
            ProtocolMessage::AudioStop(event) => Ok(ClientEffect::AudioStopped {
                event_id: event.event_id,
            }),
            ProtocolMessage::DamageDisplay(event) => Ok(ClientEffect::TransientComponent {
                component_id: event.event_id,
                decision: evaluate_experience_capability(
                    ClientCapability::UiDamageDisplay,
                    &self.accepted_capabilities,
                    true,
                ),
            }),
            ProtocolMessage::Bossbar(event) => self.apply_versioned_component(
                format!("bossbar:{}", event.boss_entity.0),
                event.revision,
                ClientCapability::UiBossbar,
                true,
            ),
            ProtocolMessage::Hologram(event) => self.apply_versioned_component(
                event.hologram_id,
                event.revision,
                ClientCapability::UiHologram,
                true,
            ),
            ProtocolMessage::Waypoint(event) => {
                let resource_ready =
                    self.resource_ready(&event.icon_resource_id, available_resources);
                self.apply_versioned_component(
                    event.waypoint_id,
                    event.revision,
                    ClientCapability::UiWaypoint,
                    resource_ready,
                )
            }
            ProtocolMessage::ModelVisibility(event) => {
                let resource_ready =
                    self.resource_ready(&event.model_resource_id, available_resources);
                self.apply_versioned_component(
                    format!("model:{}", event.entity.0),
                    event.revision,
                    ClientCapability::ModelVisibility,
                    resource_ready,
                )
            }
        }
    }

    fn apply_capabilities(
        &mut self,
        message: ProtocolMessage,
    ) -> Result<ClientEffect, ClientSessionError> {
        let ProtocolMessage::Capabilities(response) = message else {
            return Err(ClientSessionError::CapabilitiesRequired);
        };
        if response.rpg_play_allowed {
            self.accepted_capabilities = response.accepted;
            self.phase = ClientSessionPhase::Ready;
            Ok(ClientEffect::CapabilitiesAccepted)
        } else {
            self.phase = ClientSessionPhase::Rejected;
            Ok(ClientEffect::CapabilitiesRejected {
                reasons: response.error_reasons,
            })
        }
    }

    fn resource_ready(&self, resource_id: &str, available_resources: &BTreeSet<String>) -> bool {
        available_resources.contains(resource_id)
            && self
                .manifest
                .as_ref()
                .is_some_and(|manifest| manifest.find(resource_id).is_some())
    }

    fn apply_versioned_component(
        &mut self,
        component_id: String,
        revision: u64,
        required: ClientCapability,
        resources_available: bool,
    ) -> Result<ClientEffect, ClientSessionError> {
        let revision_decision = self
            .revision_gate
            .evaluate_and_record(&component_id, revision)?;
        let render_decision = evaluate_experience_capability(
            required,
            &self.accepted_capabilities,
            resources_available,
        );
        Ok(ClientEffect::VersionedComponent {
            component_id,
            revision,
            revision_decision,
            render_decision,
        })
    }
}

#[derive(Debug, Error)]
pub enum ClientSessionError {
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Experience(#[from] ExperienceError),
    #[error("capability response must be the first server message")]
    CapabilitiesRequired,
    #[error("capability response was already applied")]
    DuplicateCapabilities,
    #[error("server message uses a client-to-server message type")]
    WrongDirection,
    #[error("client session was rejected during capability negotiation")]
    SessionRejected,
    #[error("UI page is not open: {0}")]
    PageNotOpen(String),
    #[error("UI page {page_id} expected version {expected}, received {received}")]
    PageVersionMismatch {
        page_id: String,
        expected: u64,
        received: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSmokeReport {
    pub processed_frames: usize,
    pub final_phase: ClientSessionPhase,
    pub effects: Vec<ClientEffect>,
}

pub fn run_client_smoke<'a, I>(
    dispatcher: &crate::ProtocolDispatcher,
    session: &mut ClientSession,
    frames: I,
    now_unix_ms: u64,
    available_resources: &BTreeSet<String>,
) -> Result<ClientSmokeReport, ClientSmokeError>
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let mut effects = Vec::new();
    let mut processed_frames = 0;
    for frame in frames {
        let message = dispatcher.decode(frame)?;
        effects.push(session.apply(message, now_unix_ms, available_resources)?);
        processed_frames += 1;
    }
    Ok(ClientSmokeReport {
        processed_frames,
        final_phase: session.phase(),
        effects,
    })
}

#[derive(Debug, Error)]
pub enum ClientSmokeError {
    #[error(transparent)]
    Dispatch(#[from] crate::DispatchError),
    #[error(transparent)]
    Session(#[from] ClientSessionError),
}
