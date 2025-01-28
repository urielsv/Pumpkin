use std::{net::IpAddr, str::FromStr};

use crate::{
    command::{
        args::{message::MsgArgConsumer, simple::SimpleArgConsumer, Arg, ConsumedArgs},
        tree::CommandTree,
        tree_builder::argument,
        CommandError, CommandExecutor, CommandSender,
    },
    data::{
        banlist_serializer::BannedIpEntry, banned_ip_data::BANNED_IP_LIST, SaveJSONConfiguration,
    },
    server::Server,
};
use async_trait::async_trait;
use pumpkin_util::text::TextComponent;
use CommandError::InvalidConsumption;

const NAMES: [&str; 1] = ["ban-ip"];
const DESCRIPTION: &str = "bans a player-ip";

const ARG_TARGET: &str = "ip";
const ARG_REASON: &str = "reason";

async fn parse_ip(target: &str, server: &Server) -> Option<IpAddr> {
    Some(match IpAddr::from_str(target) {
        Ok(ip) => ip,
        Err(_) => server
            .get_player_by_name(target)
            .await?
            .client
            .address
            .lock()
            .await
            .ip(),
    })
}

struct BanIpNoReasonExecutor;

#[async_trait]
impl CommandExecutor for BanIpNoReasonExecutor {
    async fn execute<'a>(
        &self,
        sender: &mut CommandSender<'a>,
        server: &crate::server::Server,
        args: &ConsumedArgs<'a>,
    ) -> Result<(), CommandError> {
        let Some(Arg::Simple(target)) = args.get(&ARG_TARGET) else {
            return Err(InvalidConsumption(Some(ARG_TARGET.into())));
        };

        ban_ip(sender, server, target, None).await;
        Ok(())
    }
}

struct BanIpReasonExecutor;

#[async_trait]
impl CommandExecutor for BanIpReasonExecutor {
    async fn execute<'a>(
        &self,
        sender: &mut CommandSender<'a>,
        server: &crate::server::Server,
        args: &ConsumedArgs<'a>,
    ) -> Result<(), CommandError> {
        let Some(Arg::Simple(target)) = args.get(&ARG_TARGET) else {
            return Err(InvalidConsumption(Some(ARG_TARGET.into())));
        };

        let Some(Arg::Msg(reason)) = args.get(ARG_REASON) else {
            return Err(InvalidConsumption(Some(ARG_REASON.into())));
        };

        ban_ip(sender, server, target, Some(reason.to_string())).await;
        Ok(())
    }
}

async fn ban_ip(sender: &CommandSender<'_>, server: &Server, target: &str, reason: Option<String>) {
    let reason = reason.unwrap_or_else(|| "Banned by an operator.".to_string());

    let Some(target_ip) = parse_ip(target, server).await else {
        sender
            .send_message(TextComponent::translate(
                "commands.banip.invalid",
                [].into(),
            ))
            .await;
        return;
    };

    let mut banned_ips = BANNED_IP_LIST.write().await;

    if banned_ips.get_entry(&target_ip).is_some() {
        sender
            .send_message(TextComponent::translate("commands.banip.failed", [].into()))
            .await;
        return;
    }

    banned_ips.banned_ips.push(BannedIpEntry::new(
        target_ip,
        sender.to_string(),
        None,
        reason.clone(),
    ));

    banned_ips.save();
    drop(banned_ips);

    // Send messages
    let affected = server.get_players_by_ip(target_ip).await;
    let names = affected
        .iter()
        .map(|p| p.gameprofile.name.clone())
        .collect::<Vec<_>>()
        .join(" ");

    sender
        .send_message(TextComponent::translate(
            "commands.banip.success",
            [
                TextComponent::text(target_ip.to_string()),
                TextComponent::text(reason),
            ]
            .into(),
        ))
        .await;

    sender
        .send_message(TextComponent::translate(
            "commands.banip.info",
            [
                TextComponent::text(affected.len().to_string()),
                TextComponent::text(names),
            ]
            .into(),
        ))
        .await;

    for target in affected {
        target
            .kick(TextComponent::translate(
                "multiplayer.disconnect.ip_banned",
                [].into(),
            ))
            .await;
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION).then(
        argument(ARG_TARGET, SimpleArgConsumer)
            .execute(BanIpNoReasonExecutor)
            .then(argument(ARG_REASON, MsgArgConsumer).execute(BanIpReasonExecutor)),
    )
}
