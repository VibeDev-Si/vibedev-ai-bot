//! Forgets a room's assistant, API key included, as soon as the bot is no longer in the room.
//!
//! When the bot's own membership becomes `leave` or `ban` (it was kicked or banned, the room was shut down, or it left
//! by itself), the room's `room-local/vibedev-assistant` agent (with the encrypted API key), the text-generation handler
//! setting pointing to it, the `vibedev-assistant` access rule and the reply style are removed from the room's
//! configuration right away (see [`crate::vibedev::api::assistant::forget`]), instead of when the bot joins the room
//! again. Homeservers accept room account data for rooms the user isn't in, so this works after leaving.
//!
//! Failed attempts are retried a few times, and rooms which the bot is no longer in are checked again when it starts.

use std::time::Duration;

use mxlink::matrix_sdk::ruma::UserId;
use mxlink::matrix_sdk::ruma::events::room::member::{
    MembershipState, OriginalSyncRoomMemberEvent,
};
use mxlink::matrix_sdk::{Room, RoomState};

use crate::Bot;
use crate::entity::roomconfig::{self, RoomConfigCarrierContent};
use crate::vibedev::api::assistant;

/// How long to wait before each retry of a failed attempt.
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(10),
    Duration::from_secs(60),
];

#[derive(Debug, PartialEq)]
enum Outcome {
    Forgotten,
    NothingToForget,
    /// The bot is in the room again (invited back meanwhile): its configuration is a new one.
    BackInRoom,
}

/// Starts forgetting assistants of rooms the bot leaves, and of rooms it has already left.
pub fn attach(bot: &Bot) {
    let handler_bot = bot.clone();
    bot.matrix_link().client().add_event_handler(
        move |event: OriginalSyncRoomMemberEvent, room: Room| {
            let bot = handler_bot.clone();
            async move {
                if is_own_departure(&event, bot.user_id()) {
                    forget_with_retries(&bot, &room).await;
                }
            }
        },
    );

    let bot = bot.clone();
    tokio::spawn(async move { forget_in_rooms_left_earlier(&bot).await });
}

/// Tells whether a membership event says that the bot is no longer in the room (it left, or was kicked or banned).
fn is_own_departure(event: &OriginalSyncRoomMemberEvent, bot_user_id: &UserId) -> bool {
    event.state_key == bot_user_id
        && matches!(
            event.content.membership,
            MembershipState::Leave | MembershipState::Ban
        )
}

async fn forget_with_retries(bot: &Bot, room: &Room) {
    let room_id = room.room_id().as_str();

    for (attempt, delay) in std::iter::once(Duration::ZERO)
        .chain(RETRY_DELAYS)
        .enumerate()
    {
        tokio::time::sleep(delay).await;

        match forget(bot, room).await {
            Ok(Outcome::Forgotten) => {
                tracing::info!(
                    room_id,
                    "Forgot the VibeDev assistant (and its API key) of a room the bot is no longer in"
                );
                return;
            }
            Ok(Outcome::NothingToForget | Outcome::BackInRoom) => return,
            Err(err) => {
                tracing::warn!(
                    room_id,
                    attempt,
                    ?err,
                    "Failed to forget the VibeDev assistant of a room the bot is no longer in"
                );
            }
        }
    }

    tracing::error!(
        room_id,
        "Gave up forgetting the VibeDev assistant of a room the bot is no longer in - it will be tried again when the bot starts"
    );
}

/// Forgets the assistants of rooms the bot isn't in anymore (e.g. when forgetting failed before).
async fn forget_in_rooms_left_earlier(bot: &Bot) {
    let rooms = bot.matrix_link().client().rooms();

    for room in rooms
        .iter()
        .filter(|room| matches!(room.state(), RoomState::Left | RoomState::Banned))
    {
        match forget(bot, room).await {
            Ok(Outcome::Forgotten) => {
                tracing::info!(
                    room_id = room.room_id().as_str(),
                    "Forgot the VibeDev assistant (and its API key) of a room the bot had left"
                );
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    room_id = room.room_id().as_str(),
                    ?err,
                    "Failed to forget the VibeDev assistant of a room the bot had left"
                );
            }
        }
    }
}

async fn forget(bot: &Bot, room: &Room) -> anyhow::Result<Outcome> {
    if !matches!(room.state(), RoomState::Left | RoomState::Banned) {
        return Ok(Outcome::BackInRoom);
    }

    // Only an existing configuration is changed (reading a missing one would create one).
    if room
        .account_data_static::<RoomConfigCarrierContent>()
        .await?
        .is_none()
    {
        return Ok(Outcome::NothingToForget);
    }

    let forgotten = roomconfig::update(bot.room_config_manager(), room, |room_config| {
        assistant::forget(room_config).then_some(())
    })
    .await?
    .is_some();

    Ok(if forgotten {
        Outcome::Forgotten
    } else {
        Outcome::NothingToForget
    })
}

#[cfg(test)]
mod tests {
    use mxlink::matrix_sdk::ruma::events::room::member::OriginalSyncRoomMemberEvent;
    use mxlink::matrix_sdk::ruma::user_id;
    use serde_json::json;

    use super::is_own_departure;

    fn member_event(state_key: &str, membership: &str) -> OriginalSyncRoomMemberEvent {
        serde_json::from_value(json!({
            "type": "m.room.member",
            "event_id": "$event",
            "sender": "@admin:example.com",
            "origin_server_ts": 1,
            "state_key": state_key,
            "content": {"membership": membership},
        }))
        .unwrap()
    }

    #[test]
    fn the_bot_leaving_or_being_kicked_or_banned_is_a_departure() {
        let bot = user_id!("@vibedev-ai:example.com");

        // Leaving by itself or being kicked (both "leave"), and being banned.
        for membership in ["leave", "ban"] {
            assert!(
                is_own_departure(&member_event(bot.as_str(), membership), bot),
                "{membership}"
            );
        }

        for membership in ["join", "invite", "knock"] {
            assert!(
                !is_own_departure(&member_event(bot.as_str(), membership), bot),
                "{membership}"
            );
        }

        // Other people leaving don't matter.
        assert!(!is_own_departure(
            &member_event("@alice:example.com", "leave"),
            bot
        ));
        assert!(!is_own_departure(
            &member_event("@alice:example.com", "ban"),
            bot
        ));
    }
}
