//! The [`Backend`] of the running bot: Matrix state through matrix-sdk, configuration through baibot's own managers.

use mxlink::matrix_sdk::ruma::{RoomId, UserId};
use mxlink::matrix_sdk::{Room, RoomState};

use crate::Bot;
use crate::agent::{
    AgentDefinition, ControllerTrait, PublicIdentifier, create_from_provider_and_yaml_value_config,
};
use crate::entity::RoomConfigContext;
use crate::entity::roomconfig;
use crate::vibedev::net::{self, BaseUrl, UrlError};
use crate::vibedev::redact;

use super::assistant::{Backend, ConfigChange, MemberInfo, UpdateOutcome};
use super::types::{ApiError, ApiKey};
use super::upstream::{self, UPSTREAM_TIMEOUT, UpstreamFailure};

#[derive(Clone)]
pub struct BotBackend {
    bot: Bot,
}

impl BotBackend {
    pub fn new(bot: Bot) -> Self {
        Self { bot }
    }

    fn joined_room(&self, room_id: &RoomId) -> Result<Room, ApiError> {
        self.bot
            .matrix_link()
            .client()
            .get_room(room_id)
            .filter(|room| room.state() == RoomState::Joined)
            .ok_or_else(ApiError::bot_not_in_room)
    }

    async fn context(&self, room: &Room) -> Result<RoomConfigContext, ApiError> {
        let global_config = self.bot.global_config().await.map_err(|err| {
            tracing::warn!(?err, "Failed to load the global configuration");
            ApiError::internal("Failed to load the bot's configuration")
        })?;

        let room_config = self
            .bot
            .room_config_manager()
            .lock()
            .await
            .get_or_create_for_room(room)
            .await
            .map_err(|err| {
                tracing::warn!(
                    room_id = room.room_id().as_str(),
                    ?err,
                    "Failed to load the room configuration"
                );
                ApiError::internal("Failed to load the room's configuration")
            })?;

        Ok(RoomConfigContext::new(global_config, room_config))
    }
}

impl Backend for BotBackend {
    fn bot_joined(&self, room_id: &RoomId) -> bool {
        self.joined_room(room_id).is_ok()
    }

    async fn member(&self, room_id: &RoomId, user_id: &UserId) -> Result<MemberInfo, ApiError> {
        let room = self.joined_room(room_id)?;

        // Fetches the member list from the homeserver if it's incomplete locally (lazy loading).
        let member = room.get_member(user_id).await.map_err(|err| {
            tracing::warn!(
                room_id = room_id.as_str(),
                ?err,
                "Failed to get a room member"
            );
            ApiError::internal("Failed to read the room's members")
        })?;

        let power_levels = room.power_levels().await.map_err(|err| {
            tracing::warn!(
                room_id = room_id.as_str(),
                ?err,
                "Failed to get the room's power levels"
            );
            ApiError::internal("Failed to read the room's power levels")
        })?;

        Ok(MemberInfo {
            membership: member.map(|member| member.membership().clone()),
            power_level: power_levels.for_user(user_id),
        })
    }

    async fn load(&self, room_id: &RoomId) -> Result<RoomConfigContext, ApiError> {
        let room = self.joined_room(room_id)?;
        self.context(&room).await
    }

    async fn update(
        &self,
        room_id: &RoomId,
        change: ConfigChange,
    ) -> Result<UpdateOutcome, ApiError> {
        let room = self.joined_room(room_id)?;

        let mut changed = false;
        roomconfig::update(self.bot.room_config_manager(), &room, |room_config| {
            changed = change(room_config);
            changed.then_some(())
        })
        .await
        .map_err(|err| {
            tracing::warn!(
                room_id = room_id.as_str(),
                ?err,
                "Failed to save the room configuration"
            );
            ApiError::internal("Failed to save the room's configuration")
        })?;

        Ok(UpdateOutcome {
            changed,
            context: self.context(&room).await?,
        })
    }

    async fn check_host(&self, base_url: &BaseUrl) -> Result<(), UrlError> {
        net::check_resolves_publicly(base_url).await
    }

    async fn ping(&self, definition: &AgentDefinition) -> Result<(), ApiError> {
        let identifier = PublicIdentifier::DynamicRoomLocal(definition.id.clone());

        let agent = create_from_provider_and_yaml_value_config(
            &definition.provider,
            &identifier,
            definition.config.clone(),
        )
        .map_err(|err| {
            tracing::warn!(
                ?err,
                "Failed to create the VibeDev assistant agent for its ping"
            );
            ApiError::internal("Failed to create the agent")
        })?;

        let failure = match tokio::time::timeout(UPSTREAM_TIMEOUT, agent.controller().ping()).await
        {
            Ok(Ok(_)) => return Ok(()),
            Ok(Err(err)) => {
                let failure = upstream::classify_error(&err);

                // Logged (keys redacted), but not passed on to the caller.
                tracing::info!(
                    ?failure,
                    err = %redact::sanitize_message(&format!("{err:#}"), &[], 500),
                    "The ping of the VibeDev assistant failed"
                );

                failure
            }
            Err(_) => UpstreamFailure::Timeout,
        };

        Err(failure.into())
    }

    async fn list_models(
        &self,
        base_url: &BaseUrl,
        api_key: &ApiKey,
    ) -> Result<Vec<String>, ApiError> {
        super::models::fetch(base_url, api_key)
            .await
            .map_err(ApiError::from)
    }
}
