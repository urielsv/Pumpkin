use std::num::NonZeroU8;
use std::sync::Arc;

use crate::block::block_manager::BlockActionResult;
use crate::entity::mob;
use crate::net::PlayerConfig;
use crate::{
    command::CommandSender,
    entity::player::{ChatMode, Hand, Player},
    error::PumpkinError,
    server::Server,
    world::player_chunker,
};
use pumpkin_config::ADVANCED_CONFIG;
use pumpkin_data::entity::EntityType;
use pumpkin_data::world::CHAT;
use pumpkin_inventory::player::PlayerInventory;
use pumpkin_inventory::InventoryError;
use pumpkin_protocol::client::play::{CSetContainerSlot, CSetHeldItem};
use pumpkin_protocol::codec::slot::Slot;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::server::play::SCookieResponse as SPCookieResponse;
use pumpkin_protocol::{
    client::play::{
        Animation, CAcknowledgeBlockChange, CCommandSuggestions, CEntityAnimation, CHeadRot,
        CPingResponse, CPlayerChatMessage, CUpdateEntityPos, CUpdateEntityPosRot, CUpdateEntityRot,
        FilterType,
    },
    server::play::{
        Action, ActionType, SChatCommand, SChatMessage, SClientCommand, SClientInformationPlay,
        SCloseContainer, SCommandSuggestion, SConfirmTeleport, SInteract, SKeepAlive,
        SPickItemFromBlock, SPlayPingRequest, SPlayerAbilities, SPlayerAction, SPlayerCommand,
        SPlayerPosition, SPlayerPositionRotation, SPlayerRotation, SSetCreativeSlot, SSetHeldItem,
        SSetPlayerGround, SSwingArm, SUseItem, SUseItemOn, Status,
    },
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::color::NamedColor;
use pumpkin_util::{
    math::{vector3::Vector3, wrap_degrees},
    text::TextComponent,
    GameMode,
};
use pumpkin_world::block::block_registry::{get_block_collision_shapes, Block};
use pumpkin_world::item::item_registry::get_item_by_id;
use pumpkin_world::item::ItemStack;
use pumpkin_world::{
    block::{block_registry::get_block_by_item, BlockFace},
    entity::entity_registry::get_entity_id,
    item::item_registry::get_spawn_egg,
};
use pumpkin_world::{WORLD_LOWEST_Y, WORLD_MAX_Y};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlockPlacingError {
    BlockOutOfReach,
    InvalidBlockFace,
    BlockOutOfWorld,
    InventoryInvalid,
    InvalidGamemode,
    NoBaseBlock,
}

impl std::fmt::Display for BlockPlacingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl PumpkinError for BlockPlacingError {
    fn is_kick(&self) -> bool {
        match self {
            Self::BlockOutOfReach | Self::BlockOutOfWorld | Self::InvalidGamemode => false,
            Self::InvalidBlockFace | Self::InventoryInvalid | Self::NoBaseBlock => true,
        }
    }

    fn severity(&self) -> log::Level {
        match self {
            Self::BlockOutOfWorld | Self::InvalidGamemode | Self::NoBaseBlock => log::Level::Trace,
            Self::BlockOutOfReach | Self::InvalidBlockFace => log::Level::Warn,
            Self::InventoryInvalid => log::Level::Error,
        }
    }

    fn client_kick_reason(&self) -> Option<String> {
        match self {
            Self::BlockOutOfReach | Self::BlockOutOfWorld | Self::InvalidGamemode => None,
            Self::InvalidBlockFace => Some("Invalid block face".into()),
            Self::InventoryInvalid => Some("Held item invalid".into()),
            Self::NoBaseBlock => Some("No base block".into()),
        }
    }
}

/// Handles all Play Packets send by a real Player
/// NEVER TRUST THE CLIENT. HANDLE EVERY ERROR, UNWRAP/EXPECT ARE FORBIDDEN
impl Player {
    pub async fn handle_confirm_teleport(&self, confirm_teleport: SConfirmTeleport) {
        let mut awaiting_teleport = self.awaiting_teleport.lock().await;
        if let Some((id, position)) = awaiting_teleport.as_ref() {
            if id == &confirm_teleport.teleport_id {
                // we should set the pos now to that we requested in the teleport packet, Is may fixed issues when the client sended position packets while being teleported
                self.living_entity.set_pos(*position);

                *awaiting_teleport = None;
            } else {
                self.kick(TextComponent::text("Wrong teleport id")).await;
            }
        } else {
            self.kick(TextComponent::text(
                "Send Teleport confirm, but we did not teleport",
            ))
            .await;
        }
    }

    fn clamp_horizontal(pos: f64) -> f64 {
        pos.clamp(-3.0E7, 3.0E7)
    }

    fn clamp_vertical(pos: f64) -> f64 {
        pos.clamp(-2.0E7, 2.0E7)
    }

    pub fn handle_player_loaded(self: &Arc<Self>) {
        self.set_client_loaded(true);
    }

    pub async fn handle_position(self: &Arc<Self>, packet: SPlayerPosition) {
        if !self.has_client_loaded() {
            return;
        }
        // y = feet Y
        let position = packet.position;
        if position.x.is_nan() || position.y.is_nan() || position.z.is_nan() {
            self.kick(TextComponent::translate(
                "multiplayer.disconnect.invalid_player_movement",
                [].into(),
            ))
            .await;
            return;
        }
        let position = Vector3::new(
            Self::clamp_horizontal(position.x),
            Self::clamp_vertical(position.y),
            Self::clamp_horizontal(position.z),
        );
        let entity = &self.living_entity.entity;
        let last_pos = entity.pos.load();
        self.living_entity.set_pos(position);

        entity
            .on_ground
            .store(packet.ground, std::sync::atomic::Ordering::Relaxed);

        let entity_id = entity.entity_id;
        let Vector3 { x, y, z } = position;
        let world = &entity.world;

        // let delta = Vector3::new(x - lastx, y - lasty, z - lastz);
        // let velocity = self.velocity;

        // // Player is falling down fast, we should account for that
        // let max_speed = if self.fall_flying { 300.0 } else { 100.0 };

        // teleport when more than 8 blocks (i guess 8 blocks)
        // TODO: REPLACE * 2.0 by movement packets. see vanilla for details
        // if delta.length_squared() - velocity.length_squared() > max_speed * 2.0 {
        //     self.teleport(x, y, z, self.entity.yaw, self.entity.pitch);
        //     return;
        // }
        // send new position to all other players
        world
            .broadcast_packet_except(
                &[self.gameprofile.id],
                &CUpdateEntityPos::new(
                    entity_id.into(),
                    Vector3::new(
                        x.mul_add(4096.0, -(last_pos.x * 4096.0)) as i16,
                        y.mul_add(4096.0, -(last_pos.y * 4096.0)) as i16,
                        z.mul_add(4096.0, -(last_pos.z * 4096.0)) as i16,
                    ),
                    packet.ground,
                ),
            )
            .await;
        if !self.abilities.lock().await.flying {
            let height_difference = position.y - last_pos.y;
            self.living_entity
                .update_fall_distance(
                    height_difference,
                    packet.ground,
                    self.gamemode.load() == GameMode::Creative,
                )
                .await;
        }
        player_chunker::update_position(self).await;
    }

    pub async fn handle_position_rotation(self: &Arc<Self>, packet: SPlayerPositionRotation) {
        if !self.has_client_loaded() {
            return;
        }
        // y = feet Y
        let position = packet.position;
        if position.x.is_nan()
            || position.y.is_nan()
            || position.z.is_nan()
            || packet.yaw.is_infinite()
            || packet.pitch.is_infinite()
        {
            self.kick(TextComponent::translate(
                "multiplayer.disconnect.invalid_player_movement",
                [].into(),
            ))
            .await;
            return;
        }

        let position = Vector3::new(
            Self::clamp_horizontal(position.x),
            Self::clamp_vertical(position.y),
            Self::clamp_horizontal(position.z),
        );
        let entity = &self.living_entity.entity;
        let last_pos = entity.pos.load();
        self.living_entity.set_pos(position);

        entity
            .on_ground
            .store(packet.ground, std::sync::atomic::Ordering::Relaxed);

        entity.set_rotation(
            wrap_degrees(packet.yaw) % 360.0,
            wrap_degrees(packet.pitch).clamp(-90.0, 90.0) % 360.0,
        );

        let entity_id = entity.entity_id;
        let Vector3 { x, y, z } = position;

        let yaw = (entity.yaw.load() * 256.0 / 360.0).rem_euclid(256.0);
        let pitch = (entity.pitch.load() * 256.0 / 360.0).rem_euclid(256.0);
        // let head_yaw = (entity.head_yaw * 256.0 / 360.0).floor();
        let world = &entity.world;

        // let delta = Vector3::new(x - lastx, y - lasty, z - lastz);
        // let velocity = self.velocity;

        // // Player is falling down fast, we should account for that
        // let max_speed = if self.fall_flying { 300.0 } else { 100.0 };

        // // teleport when more than 8 blocks (i guess 8 blocks)
        // // TODO: REPLACE * 2.0 by movement packets. see vanilla for details
        // if delta.length_squared() - velocity.length_squared() > max_speed * 2.0 {
        //     self.teleport(x, y, z, yaw, pitch);
        //     return;
        // }
        // send new position to all other players

        world
            .broadcast_packet_except(
                &[self.gameprofile.id],
                &CUpdateEntityPosRot::new(
                    entity_id.into(),
                    Vector3::new(
                        x.mul_add(4096.0, -(last_pos.x * 4096.0)) as i16,
                        y.mul_add(4096.0, -(last_pos.y * 4096.0)) as i16,
                        z.mul_add(4096.0, -(last_pos.z * 4096.0)) as i16,
                    ),
                    yaw as u8,
                    pitch as u8,
                    packet.ground,
                ),
            )
            .await;
        world
            .broadcast_packet_except(
                &[self.gameprofile.id],
                &CHeadRot::new(entity_id.into(), yaw as u8),
            )
            .await;
        if !self.abilities.lock().await.flying {
            let height_difference = position.y - last_pos.y;
            self.living_entity
                .update_fall_distance(
                    height_difference,
                    packet.ground,
                    self.gamemode.load() == GameMode::Creative,
                )
                .await;
        }
        player_chunker::update_position(self).await;
    }

    pub async fn handle_rotation(&self, rotation: SPlayerRotation) {
        if !self.has_client_loaded() {
            return;
        }
        if !rotation.yaw.is_finite() || !rotation.pitch.is_finite() {
            self.kick(TextComponent::translate(
                "multiplayer.disconnect.invalid_player_movement",
                [].into(),
            ))
            .await;
            return;
        }
        let entity = &self.living_entity.entity;
        entity
            .on_ground
            .store(rotation.ground, std::sync::atomic::Ordering::Relaxed);
        entity.set_rotation(
            wrap_degrees(rotation.yaw) % 360.0,
            wrap_degrees(rotation.pitch).clamp(-90.0, 90.0) % 360.0,
        );
        // send new position to all other players
        let entity_id = entity.entity_id;
        let yaw = (entity.yaw.load() * 256.0 / 360.0).rem_euclid(256.0);
        let pitch = (entity.pitch.load() * 256.0 / 360.0).rem_euclid(256.0);
        // let head_yaw = modulus(entity.head_yaw * 256.0 / 360.0, 256.0);

        let world = &entity.world;
        let packet =
            CUpdateEntityRot::new(entity_id.into(), yaw as u8, pitch as u8, rotation.ground);
        world
            .broadcast_packet_except(&[self.gameprofile.id], &packet)
            .await;
        let packet = CHeadRot::new(entity_id.into(), yaw as u8);
        world
            .broadcast_packet_except(&[self.gameprofile.id], &packet)
            .await;
    }

    pub fn handle_chat_command(self: &Arc<Self>, server: &Arc<Server>, command: &SChatCommand) {
        let player_clone = self.clone();
        let server_clone = server.clone();
        let command_clone = command.command.clone();
        // Some commands can take a long time to execute. If they do, they block packet processing for the player
        // Thats why we will spawn a task instead
        tokio::spawn(async move {
            let dispatcher = server_clone.command_dispatcher.read().await;
            dispatcher
                .handle_command(
                    &mut CommandSender::Player(player_clone),
                    &server_clone,
                    &command_clone,
                )
                .await;
        });

        if ADVANCED_CONFIG.commands.log_console {
            log::info!(
                "Player ({}): executed command /{}",
                self.gameprofile.name,
                command.command
            );
        }
    }

    pub fn handle_player_ground(&self, ground: &SSetPlayerGround) {
        self.living_entity
            .entity
            .on_ground
            .store(ground.on_ground, std::sync::atomic::Ordering::Relaxed);
    }

    async fn update_single_slot(
        &self,
        inventory: &mut tokio::sync::MutexGuard<'_, PlayerInventory>,
        slot: i16,
        slot_data: Slot,
    ) {
        inventory.state_id += 1;
        let dest_packet = CSetContainerSlot::new(0, inventory.state_id as i32, slot, &slot_data);
        self.client.send_packet(&dest_packet).await;

        if inventory
            .set_slot(slot as usize, slot_data.to_item(), false)
            .is_err()
        {
            log::error!("Pick item set slot error!");
        }
    }

    pub async fn handle_pick_item_from_block(&self, pick_item: SPickItemFromBlock) {
        if !self.can_interact_with_block_at(&pick_item.pos, 1.0) {
            return;
        }

        let Ok(block) = self.world().get_block(&pick_item.pos).await else {
            return;
        };

        if block.item_id == 0 {
            // Invalid block id (blocks such as tall seagrass)
            return;
        }

        let mut inventory = self.inventory().lock().await;

        let source_slot = inventory.get_slot_with_item(block.item_id);
        let mut dest_slot = inventory.get_pick_item_hotbar_slot() as usize;

        let dest_slot_data = match inventory.get_slot(dest_slot + 36) {
            Ok(Some(stack)) => Slot::from(&*stack),
            _ => Slot::from(None),
        };

        // Early return if no source slot and not in creative mode
        if source_slot.is_none() && self.gamemode.load() != GameMode::Creative {
            return;
        }

        match source_slot {
            Some(slot_index) if (36..=44).contains(&slot_index) => {
                // Case where item is in hotbar
                dest_slot = slot_index - 36;
            }
            Some(slot_index) => {
                // Case where item is in inventory

                // Update destination slot
                let source_slot_data = match inventory.get_slot(slot_index) {
                    Ok(Some(stack)) => Slot::from(&*stack),
                    _ => return,
                };
                self.update_single_slot(&mut inventory, dest_slot as i16 + 36, source_slot_data)
                    .await;

                // Update source slot
                self.update_single_slot(&mut inventory, slot_index as i16, dest_slot_data)
                    .await;
            }
            None if self.gamemode.load() == GameMode::Creative => {
                // Case where item is not present, if in creative mode create the item
                let item_stack = ItemStack::new(1, block.item_id);
                let slot_data = Slot::from(&item_stack);
                self.update_single_slot(&mut inventory, dest_slot as i16 + 36, slot_data)
                    .await;

                // Check if there is any empty slot in the player inventory
                if let Some(slot_index) = inventory.get_empty_slot() {
                    inventory.state_id += 1;
                    self.update_single_slot(&mut inventory, slot_index as i16, dest_slot_data)
                        .await;
                }
            }
            _ => return,
        }

        // Update held item
        inventory.set_selected(dest_slot as u32);
        self.client
            .send_packet(&CSetHeldItem::new(dest_slot as i8))
            .await;
    }

    // pub fn handle_pick_item_from_entity(&self, _pick_item: SPickItemFromEntity) {
    //     // TODO: Implement and merge any redundant code with pick_item_from_block
    // }

    pub async fn handle_player_command(&self, command: SPlayerCommand) {
        if command.entity_id != self.entity_id().into() {
            return;
        }
        if !self.has_client_loaded() {
            return;
        }

        if let Ok(action) = Action::try_from(command.action.0) {
            let entity = &self.living_entity.entity;
            match action {
                pumpkin_protocol::server::play::Action::StartSneaking => {
                    if !entity.sneaking.load(std::sync::atomic::Ordering::Relaxed) {
                        entity.set_sneaking(true).await;
                    }
                }
                pumpkin_protocol::server::play::Action::StopSneaking => {
                    if entity.sneaking.load(std::sync::atomic::Ordering::Relaxed) {
                        entity.set_sneaking(false).await;
                    }
                }
                pumpkin_protocol::server::play::Action::StartSprinting => {
                    if !entity.sprinting.load(std::sync::atomic::Ordering::Relaxed) {
                        entity.set_sprinting(true).await;
                    }
                }
                pumpkin_protocol::server::play::Action::StopSprinting => {
                    if entity.sprinting.load(std::sync::atomic::Ordering::Relaxed) {
                        entity.set_sprinting(false).await;
                    }
                }
                pumpkin_protocol::server::play::Action::LeaveBed
                | pumpkin_protocol::server::play::Action::StartHorseJump
                | pumpkin_protocol::server::play::Action::StopHorseJump
                | pumpkin_protocol::server::play::Action::OpenVehicleInventory => {
                    log::debug!("todo");
                }
                pumpkin_protocol::server::play::Action::StartFlyingElytra => {
                    let fall_flying = entity.check_fall_flying();
                    if entity
                        .fall_flying
                        .load(std::sync::atomic::Ordering::Relaxed)
                        != fall_flying
                    {
                        entity.set_fall_flying(fall_flying).await;
                    }
                } // TODO
            }
        } else {
            self.kick(TextComponent::text("Invalid player command"))
                .await;
        }
    }

    pub async fn handle_swing_arm(&self, swing_arm: SSwingArm) {
        let animation = match swing_arm.hand.0 {
            0 => Animation::SwingMainArm,
            1 => Animation::SwingOffhand,
            _ => {
                self.kick(TextComponent::text("Invalid hand")).await;
                return;
            }
        };
        // Invert hand if player is left handed
        let animation = match self.config.lock().await.main_hand {
            Hand::Left => match animation {
                Animation::SwingMainArm => Animation::SwingOffhand,
                Animation::SwingOffhand => Animation::SwingMainArm,
                _ => unreachable!(),
            },
            Hand::Right => animation,
        };

        let id = self.entity_id();
        let world = self.world();
        world
            .broadcast_packet_except(
                &[self.gameprofile.id],
                &CEntityAnimation::new(id.into(), animation as u8),
            )
            .await;
    }

    pub async fn handle_chat_message(&self, chat_message: SChatMessage) {
        let message = chat_message.message;
        if message.len() > 256 {
            self.kick(TextComponent::text("Oversized message")).await;
            return;
        }

        if message.chars().any(|c| c == '§' || c < ' ' || c == '\x7F') {
            self.kick(TextComponent::translate(
                "multiplayer.disconnect.illegal_characters",
                [].into(),
            ))
            .await;
            return;
        }

        let gameprofile = &self.gameprofile;
        log::info!("<chat>{}: {}", gameprofile.name, message);

        let entity = &self.living_entity.entity;
        let world = &entity.world;
        world
            .broadcast_packet_all(&CPlayerChatMessage::new(
                gameprofile.id,
                1.into(),
                chat_message.signature.as_deref(),
                &message,
                chat_message.timestamp,
                chat_message.salt,
                &[],
                Some(TextComponent::text(message.clone())),
                FilterType::PassThrough,
                (CHAT + 1).into(),
                TextComponent::text(gameprofile.name.clone()),
                None,
            ))
            .await;

        /* server.broadcast_packet(
            self,
            &CDisguisedChatMessage::new(
                TextComponent::from(message.clone()),
                VarInt(0),
               gameprofile.name.clone().into(),
                None,
            ),
        ) */
    }

    pub async fn handle_client_information(
        self: &Arc<Self>,
        client_information: SClientInformationPlay,
    ) {
        if let (Ok(main_hand), Ok(chat_mode)) = (
            Hand::try_from(client_information.main_hand.0),
            ChatMode::try_from(client_information.chat_mode.0),
        ) {
            if client_information.view_distance <= 0 {
                self.kick(TextComponent::text(
                    "Cannot have zero or negative view distance!",
                ))
                .await;
                return;
            }

            let (update_skin, update_watched) = {
                let mut config = self.config.lock().await;
                let update_skin = config.main_hand != main_hand
                    || config.skin_parts != client_information.skin_parts;

                let old_view_distance = config.view_distance;

                let update_watched =
                    if old_view_distance.get() == client_information.view_distance as u8 {
                        false
                    } else {
                        log::debug!(
                            "Player {} ({}) updated render distance: {} -> {}.",
                            self.gameprofile.name,
                            self.client.id,
                            old_view_distance,
                            client_information.view_distance
                        );

                        true
                    };

                *config = PlayerConfig {
                    locale: client_information.locale,
                    // A Negative view distance would be impossible and make no sense right ?, Mojang: Lets make is signed :D
                    view_distance: unsafe {
                        NonZeroU8::new_unchecked(client_information.view_distance as u8)
                    },
                    chat_mode,
                    chat_colors: client_information.chat_colors,
                    skin_parts: client_information.skin_parts,
                    main_hand,
                    text_filtering: client_information.text_filtering,
                    server_listing: client_information.server_listing,
                };
                (update_skin, update_watched)
            };

            if update_watched {
                player_chunker::update_position(self).await;
            }

            if update_skin {
                log::debug!(
                    "Player {} ({}) updated their skin.",
                    self.gameprofile.name,
                    self.client.id,
                );
                self.update_client_information().await;
            }
        } else {
            self.kick(TextComponent::text("Invalid hand or chat type"))
                .await;
        }
    }

    pub async fn handle_client_status(self: &Arc<Self>, client_status: SClientCommand) {
        match client_status.action_id.0 {
            0 => {
                // Perform Respawn
                if self.living_entity.health.load() > 0.0 {
                    return;
                }
                self.world().respawn_player(&self.clone(), false).await;

                // Restore abilities based on gamemode after respawn
                let mut abilities = self.abilities.lock().await;
                abilities.set_for_gamemode(self.gamemode.load());
                drop(abilities);
                self.send_abilities_update().await;
            }
            1 => {
                // request stats
                log::debug!("todo");
            }
            _ => {
                self.kick(TextComponent::text("Invalid client status"))
                    .await;
            }
        };
    }

    pub async fn handle_interact(&self, interact: SInteract) {
        if !self.has_client_loaded() {
            return;
        }

        let sneaking = interact.sneaking;
        let entity = &self.living_entity.entity;
        if entity.sneaking.load(std::sync::atomic::Ordering::Relaxed) != sneaking {
            entity.set_sneaking(sneaking).await;
        }
        let Ok(action) = ActionType::try_from(interact.typ.0) else {
            self.kick(TextComponent::text("Invalid action type")).await;
            return;
        };

        match action {
            ActionType::Attack => {
                let entity_id = interact.entity_id;
                let config = &ADVANCED_CONFIG.pvp;
                // TODO: do validation and stuff
                if !config.enabled {
                    return;
                }

                let world = &entity.world;
                let player_victim = world.get_player_by_entityid(entity_id.0).await;
                let entity_victim = world.get_living_entity_by_entityid(entity_id.0).await;
                if let Some(player_victim) = player_victim {
                    if player_victim.living_entity.health.load() <= 0.0 {
                        // you can trigger this from a non-modded / innocent client client,
                        // so we shouldn't kick the player
                        return;
                    }
                    self.attack(&player_victim).await;
                } else if let Some(entity_victim) = entity_victim {
                    if entity_victim.health.load() <= 0.0 {
                        return;
                    }
                    entity_victim.kill().await;
                    world.clone().remove_mob_entity(entity_victim).await;
                    // TODO: block entities should be checked here (signs)
                } else {
                    log::error!(
                        "Player id {} interacted with entity id {} which was not found.",
                        self.entity_id(),
                        entity_id.0
                    );
                    self.kick(TextComponent::translate(
                        "multiplayer.disconnect.invalid_entity_attacked",
                        [].into(),
                    ))
                    .await;
                    return;
                };

                if entity_id.0 == self.entity_id() {
                    // this, however, can't be triggered from a non-modded client.
                    self.kick(TextComponent::text("You can't attack yourself"))
                        .await;
                }
            }
            ActionType::Interact | ActionType::InteractAt => {
                log::debug!("todo");
            }
        }
    }

    pub async fn handle_player_action(
        self: Arc<Self>,
        player_action: SPlayerAction,
        server: &Server,
    ) {
        if !self.has_client_loaded() {
            return;
        }
        match Status::try_from(player_action.status.0) {
            Ok(status) => match status {
                Status::StartedDigging => {
                    if !self.can_interact_with_block_at(&player_action.location, 1.0) {
                        log::warn!(
                            "Player {0} tried to interact with block out of reach at {1}",
                            self.gameprofile.name,
                            player_action.location
                        );
                        return;
                    }
                    // TODO: do validation
                    // TODO: Config
                    if self.gamemode.load() == GameMode::Creative {
                        let location = player_action.location;
                        // Block break & block break sound
                        let entity = &self.living_entity.entity;
                        let world = &entity.world;
                        let block = world.get_block(&location).await;

                        world.break_block(&location, Some(self.clone())).await;

                        if let Ok(block) = block {
                            server
                                .block_manager
                                .on_broken(block, &self, location, server)
                                .await;
                        }
                    }
                }
                Status::CancelledDigging => {
                    if !self.can_interact_with_block_at(&player_action.location, 1.0) {
                        log::warn!(
                            "Player {0} tried to interact with block out of reach at {1}",
                            self.gameprofile.name,
                            player_action.location
                        );
                        return;
                    }
                    self.current_block_destroy_stage
                        .store(0, std::sync::atomic::Ordering::Relaxed);
                }
                Status::FinishedDigging => {
                    // TODO: do validation
                    let location = player_action.location;
                    if !self.can_interact_with_block_at(&location, 1.0) {
                        log::warn!(
                            "Player {0} tried to interact with block out of reach at {1}",
                            self.gameprofile.name,
                            player_action.location
                        );
                        return;
                    }
                    // Block break & block break sound
                    let entity = &self.living_entity.entity;
                    let world = &entity.world;
                    let block = world.get_block(&location).await;

                    world.break_block(&location, Some(self.clone())).await;

                    if let Ok(block) = block {
                        server
                            .block_manager
                            .on_broken(block, &self, location, server)
                            .await;
                    }
                }
                Status::DropItemStack
                | Status::DropItem
                | Status::ShootArrowOrFinishEating
                | Status::SwapItem => {
                    log::debug!("todo");
                }
            },
            Err(_) => self.kick(TextComponent::text("Invalid status")).await,
        }

        self.client
            .send_packet(&CAcknowledgeBlockChange::new(player_action.sequence))
            .await;
    }

    pub async fn handle_keep_alive(&self, keep_alive: SKeepAlive) {
        if self
            .wait_for_keep_alive
            .load(std::sync::atomic::Ordering::Relaxed)
            && keep_alive.keep_alive_id
                == self
                    .keep_alive_id
                    .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.wait_for_keep_alive
                .store(false, std::sync::atomic::Ordering::Relaxed);
        } else {
            self.kick(TextComponent::text("Timeout")).await;
        }
    }

    pub async fn handle_player_abilities(&self, player_abilities: SPlayerAbilities) {
        let mut abilities = self.abilities.lock().await;

        // Set the flying ability
        let flying = player_abilities.flags & 0x02 != 0 && abilities.allow_flying;
        if flying {
            self.living_entity.fall_distance.store(0.0);
        }
        abilities.flying = flying;
    }

    pub async fn handle_play_ping_request(&self, request: SPlayPingRequest) {
        self.client
            .send_packet(&CPingResponse::new(request.payload))
            .await;
    }

    pub async fn handle_use_item_on(
        &self,
        use_item_on: SUseItemOn,
        server: &Arc<Server>,
    ) -> Result<(), Box<dyn PumpkinError>> {
        if !self.has_client_loaded() {
            return Ok(());
        }

        let location = use_item_on.location;
        let mut should_try_decrement = false;

        if !self.can_interact_with_block_at(&location, 1.0) {
            // TODO: maybe log?
            return Err(BlockPlacingError::BlockOutOfReach.into());
        }

        let Ok(face) = BlockFace::try_from(use_item_on.face.0) else {
            return Err(BlockPlacingError::InvalidBlockFace.into());
        };

        let mut inventory = self.inventory().lock().await;
        let entity = &self.living_entity.entity;
        let world = &entity.world;
        let slot_id = inventory.get_selected();
        let mut state_id = inventory.state_id;
        let item_slot = *inventory.held_item_mut();
        drop(inventory);

        let Ok(block) = world.get_block(&location).await else {
            return Err(BlockPlacingError::NoBaseBlock.into());
        };

        let Some(stack) = item_slot else {
            if !self
                .living_entity
                .entity
                .sneaking
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                // Using block with empty hand
                server
                    .block_manager
                    .on_use(block, self, location, server)
                    .await;
            }
            return Ok(());
        };
        let Some(item) = get_item_by_id(stack.item_id) else {
            // If there is an item stack, there should be an item
            return Err(BlockPlacingError::InventoryInvalid.into());
        };

        if !self
            .living_entity
            .entity
            .sneaking
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let action_result = server
                .block_manager
                .on_use_with_item(block, self, location, item, server)
                .await;
            match action_result {
                BlockActionResult::Continue => {}
                BlockActionResult::Consume => {
                    return Ok(());
                }
            }
        }
        // check if item is a block, Because Not every item can be placed :D
        if let Some(block) = get_block_by_item(stack.item_id) {
            should_try_decrement = self
                .run_is_block_place(block.clone(), server, use_item_on, location, &face)
                .await?;
        }
        // check if item is a spawn egg
        if let Some(item_t) = get_spawn_egg(stack.item_id) {
            should_try_decrement = self
                .run_is_spawn_egg(item_t, server, location, &face)
                .await?;
        };

        if should_try_decrement {
            // TODO: Config
            // Decrease Block count
            if self.gamemode.load() != GameMode::Creative {
                let mut inventory = self.inventory().lock().await;
                let item_slot = inventory.held_item_mut();
                // This should never be possible
                let Some(item_stack) = item_slot else {
                    return Err(BlockPlacingError::InventoryInvalid.into());
                };
                item_stack.item_count -= 1;
                if item_stack.item_count == 0 {
                    *item_slot = None;
                }

                // TODO: this should be by use item on not currently selected as they might be different
                let _ = self
                    .handle_decrease_item(server, slot_id as i16, item_slot.as_ref(), &mut state_id)
                    .await;
            }
        }

        Ok(())
    }

    pub fn handle_use_item(&self, _use_item: &SUseItem) {
        if !self.has_client_loaded() {
            return;
        }
        // TODO: handle packet correctly
        log::error!("An item was used(SUseItem), but the packet is not implemented yet");
    }

    pub async fn handle_set_held_item(&self, held: SSetHeldItem) {
        let slot = held.slot;
        if !(0..=8).contains(&slot) {
            self.kick(TextComponent::text("Invalid held slot")).await;
            return;
        }
        self.inventory().lock().await.set_selected(slot as u32);
    }

    pub async fn handle_set_creative_slot(
        &self,
        packet: SSetCreativeSlot,
    ) -> Result<(), InventoryError> {
        if self.gamemode.load() != GameMode::Creative {
            return Err(InventoryError::PermissionError);
        }
        let valid_slot = packet.slot >= 0 && packet.slot <= 45;
        if valid_slot {
            self.inventory().lock().await.set_slot(
                packet.slot as usize,
                packet.clicked_item.to_item(),
                true,
            )?;
        };
        // TODO: The Item was dropped per drag and drop,
        Ok(())
    }

    // TODO:
    // This function will in the future be used to keep track of if the client is in a valid state.
    // But this is not possible yet
    pub async fn handle_close_container(&self, server: &Server, _packet: SCloseContainer) {
        // TODO: This should check if player sent this packet before
        // let Some(_window_type) = WindowType::from_i32(packet.window_id.0) else {
        //     log::info!("Closed ID: {}", packet.window_id.0);
        //     self.kick(TextComponent::text("Invalid window ID")).await;
        //     return;
        // };
        // window_id 0 represents both 9x1 Generic AND inventory here
        let mut inventory = self.inventory().lock().await;

        inventory.state_id = 0;
        let open_container = self.open_container.load();
        if let Some(id) = open_container {
            let mut open_containers = server.open_containers.write().await;
            if let Some(container) = open_containers.get_mut(&id) {
                // If container contains both a location and a type, run the on_close block_manager handler
                if let Some(pos) = container.get_location() {
                    if let Some(block) = container.get_block() {
                        server
                            .block_manager
                            .on_close(&block, self, pos, server, container) //block, self, location, server)
                            .await;
                    }
                }
                // Remove the player from the container
                container.remove_player(self.entity_id());
            }
            self.open_container.store(None);
        }
    }

    pub async fn handle_command_suggestion(
        self: &Arc<Self>,
        packet: SCommandSuggestion,
        server: &Arc<Server>,
    ) {
        let mut src = CommandSender::Player(self.clone());
        let Some(cmd) = &packet.command.get(1..) else {
            return;
        };

        let Some((last_word_start, _)) = cmd.char_indices().rfind(|(_, c)| c.is_whitespace())
        else {
            return;
        };

        let dispatcher = server.command_dispatcher.read().await;
        let suggestions = dispatcher.find_suggestions(&mut src, server, cmd).await;

        let response = CCommandSuggestions::new(
            packet.id,
            (last_word_start + 2).into(),
            (cmd.len() - last_word_start - 1).into(),
            suggestions,
        );

        self.client.send_packet(&response).await;
    }

    pub fn handle_cookie_response(&self, packet: SPCookieResponse) {
        // TODO: allow plugins to access this
        log::debug!(
            "Received cookie_response[play]: key: \"{}\", has_payload: \"{}\", payload_length: \"{}\"",
            packet.key.to_string(),
            packet.has_payload,
            packet.payload_length.unwrap_or(VarInt::from(0)).0
        );
    }

    async fn run_is_spawn_egg(
        &self,
        item_t: String,
        server: &Server,
        location: BlockPos,
        face: &BlockFace,
    ) -> Result<bool, Box<dyn PumpkinError>> {
        // checks if spawn egg has a corresponding entity name
        if let Some(spawn_item_id) = get_entity_id(&item_t) {
            let world_pos = BlockPos(location.0 + face.to_offset());
            // align position like Vanilla does
            let pos = Vector3::new(
                f64::from(world_pos.0.x) + 0.5,
                f64::from(world_pos.0.y),
                f64::from(world_pos.0.z) + 0.5,
            );
            // create rotation like Vanilla
            let yaw = wrap_degrees(rand::random::<f32>() * 360.0) % 360.0;

            let world = self.world();
            // create new mob and uuid based on spawn egg id
            let (mob, uuid) = mob::from_type(
                EntityType::from_raw(*spawn_item_id).unwrap(),
                server,
                pos,
                world,
            )
            .await;

            // set the rotation
            mob.living_entity.entity.set_rotation(yaw, 0.0);

            // broadcast new mob to all players
            world
                .broadcast_packet_all(&mob.living_entity.entity.create_spawn_packet(uuid))
                .await;

            // TODO: send/configure additional commands/data based on type of entity (horse, slime, etc)
        } else {
            // TODO: maybe include additional error types
            return Ok(false);
        };

        Ok(true)
    }

    async fn run_is_block_place(
        &self,
        block: Block,
        server: &Server,
        use_item_on: SUseItemOn,
        location: BlockPos,
        face: &BlockFace,
    ) -> Result<bool, Box<dyn PumpkinError>> {
        let entity = &self.living_entity.entity;
        let world = &entity.world;

        // check block under the world
        if location.0.y + face.to_offset().y < WORLD_LOWEST_Y.into() {
            self.client
                .send_packet(&CAcknowledgeBlockChange::new(use_item_on.sequence))
                .await;
            return Err(BlockPlacingError::BlockOutOfWorld.into());
        }

        //check max world build height
        if location.0.y + face.to_offset().y >= WORLD_MAX_Y.into() {
            self.send_system_message_raw(
                &TextComponent::translate(
                    "build.tooHigh",
                    vec![TextComponent::text((WORLD_MAX_Y - 1).to_string())],
                )
                .color_named(NamedColor::Red),
                true,
            )
            .await;
            self.client
                .send_packet(&CAcknowledgeBlockChange::new(use_item_on.sequence))
                .await;
            return Err(BlockPlacingError::BlockOutOfWorld.into());
        }

        match self.gamemode.load() {
            GameMode::Spectator | GameMode::Adventure => {
                self.client
                    .send_packet(&CAcknowledgeBlockChange::new(use_item_on.sequence))
                    .await;
                return Err(BlockPlacingError::InvalidGamemode.into());
            }
            _ => {}
        }

        let clicked_world_pos = BlockPos(location.0);
        let clicked_block_state = world.get_block_state(&clicked_world_pos).await?;

        let world_pos = if clicked_block_state.replaceable {
            clicked_world_pos
        } else {
            let world_pos = BlockPos(location.0 + face.to_offset());
            let previous_block_state = world.get_block_state(&world_pos).await?;

            if !previous_block_state.replaceable {
                return Ok(true);
            }

            world_pos
        };

        // To this point we must have the new block state
        let block_bounding_box =
            get_block_collision_shapes(block.default_state_id).unwrap_or_default();
        let mut intersects = false;
        for player in world.get_nearby_players(entity.pos.load(), 20.0).await {
            let bounding_box = player.1.living_entity.entity.bounding_box.load();
            if bounding_box.intersects_block(&world_pos, &block_bounding_box) {
                intersects = true;
            }
        }
        if !intersects {
            world
                .set_block_state(&world_pos, block.default_state_id)
                .await;
            server
                .block_manager
                .on_placed(&block, self, world_pos, server)
                .await;
        }
        self.client
            .send_packet(&CAcknowledgeBlockChange::new(use_item_on.sequence))
            .await;
        Ok(true)
    }
}
